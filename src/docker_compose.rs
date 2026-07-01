use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use anyhow::Context;
use serde::Serialize;
use crate::resolved_spec::{EnvironmentResolvedSpec, ServiceResolvedSpec};
use crate::spec;
use crate::spec::{EnvVariable, SecretMount, ServiceCommand, ServiceType, ServiceVolumeType};

#[derive(Serialize)]
pub struct DockerCompose {
    pub services: HashMap<String, DockerService>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub networks: HashMap<String, DockerComposeNetwork>,
}

#[derive(Serialize)]
pub struct DockerService {
    pub image: String,
    pub container_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<ServiceCommand>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env_file: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub environment: HashMap<String, String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub networks: HashMap<String, ServiceNetwork>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deploy: Option<DeployConfig>,
}

#[derive(Serialize)]
pub struct DeployConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<RestartPolicy>,
}

#[derive(Serialize)]
pub struct RestartPolicy {
    pub condition: String,
}

#[derive(Serialize)]
pub struct DockerComposeNetwork {
    pub external: bool,
    pub name: String,
}

#[derive(Serialize)]
pub struct ServiceNetwork {
    pub aliases: Vec<String>,
}

pub fn prepare_service(service: &ServiceResolvedSpec, spec: &EnvironmentResolvedSpec, output_dir: &Path) -> anyhow::Result<DockerService> {
    let svc_dir = output_dir.join(service.full_name.clone());
    fs::create_dir_all(&svc_dir).context("Failed to create service directory")?;

    // Generate .env files
    let env_path = svc_dir.join(".env".to_string());
    write_env_file(&env_path, &service.environment_variables)?;

    // A host-run service with a `working_dir` gets its `.env` written into that
    // directory by `write_working_dir`, so skip the in-tree `undockerized.env`.
    if spec.env_type == spec::DeploymentEnvType::Local && service.working_dir.is_none() {
        let undoc_env_path = svc_dir.join("undockerized.env".to_string());
        write_env_file(&undoc_env_path, &service.undockerized_environment_variables)?;
    }

    let mut volumes = Vec::new();
    let mut environment = HashMap::new();

    for volume in &service.volumes {
        match &volume.name {
            ServiceVolumeType::Named(name) => {
                volumes.push(format!("./volumes/{}:{}",  name, volume.mount_path));
            }
            ServiceVolumeType::Path(from_path) => {
                volumes.push(format!("{}:{}",  from_path, volume.mount_path));
            }
        }
    }

    // Configs
    for config_option in &service.configs {
        if let Some(config_spec) = spec.current_deployment.configs.iter().find(|c| c.name == config_option.config_name) {
            let rel_path = config_option.mount_path.trim_start_matches('/');
            let host_path = svc_dir.join(rel_path);

            let is_file_mount = if config_spec.files.len() == 1 {
                let file = &config_spec.files[0];
                let mount_filename = Path::new(&config_option.mount_path).file_name().unwrap_or_default().to_string_lossy();
                mount_filename == file.name
            } else {
                false
            };

            if is_file_mount {
                if let Some(parent) = host_path.parent() {
                    fs::create_dir_all(parent).context("Failed to create config parent directory")?;
                }
                fs::write(&host_path, &config_spec.files[0].content).context("Failed to write config file")?;

                // Use forward slashes for docker-compose
                let rel_path_str = rel_path.replace("\\", "/");
                volumes.push(format!("./{}/{}:{}", service.full_name, rel_path_str, config_option.mount_path));
            } else {
                fs::create_dir_all(&host_path).context("Failed to create config directory")?;
                for file in &config_spec.files {
                    let p = host_path.join(&file.name);
                    fs::write(&p, &file.content).context("Failed to write config file inside dir")?;
                }

                let rel_path_str = rel_path.replace("\\", "/");
                volumes.push(format!("./{}/{}:{}", service.full_name, rel_path_str, config_option.mount_path));
            }
        } else {
            eprintln!("Warning: Config {} not found for service {}", config_option.config_name, service.full_name);
        }
    }


    // Secrets
    for secret_option in &service.secrets {
        if let Some(secret_spec) =  spec.current_deployment.secrets.iter().find(|s| s.name == secret_option.name) {
            match &secret_option.mount {
                SecretMount::EnvVariable(var_name) => {
                    environment.insert(var_name.clone(), secret_spec.value.clone());
                },

                SecretMount::FilePath(mount_path) => {
                    let rel_path = mount_path.trim_start_matches('/');
                    let host_path = svc_dir.join(rel_path);

                    if let Some(parent) = host_path.parent() {
                        fs::create_dir_all(parent).context("Failed to create secret parent directory")?;
                    }
                    fs::write(&host_path, &secret_spec.value).context("Failed to write secret file")?;

                    let rel_path_str = rel_path.replace("\\", "/");
                    volumes.push(format!("./{}/{}:{}", service.full_name, rel_path_str, mount_path));
                }
            }
        } else {
            eprintln!("Warning: Secret {} not found for service {}", secret_option.name, service.full_name);
        }
    }

    // Ports
    let ports = service.ports.iter()
        .map(|port| format!("{}:{}", port.external, port.internal))
        .collect();

    let deploy_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    environment.insert("DEPLOY_DATE".to_string(), deploy_date.to_string());

    // Job services run to completion and must not be restarted by Swarm.
    // Swarm's default restart policy is `condition: any`, which would keep
    // re-running a job after it exits, so disable restarts explicitly.
    let deploy = match service.service_type {
        ServiceType::Job => Some(DeployConfig {
            restart_policy: Some(RestartPolicy {
                condition: "none".to_string(),
            }),
        }),
        _ => None,
    };

    Ok(DockerService {
        image: service.image.clone(),
        container_name: service.full_name.clone(),
        command: service.command.clone(),
        ports,
        volumes,
        env_file: vec![format!("./{}/.env", service.full_name)],
        environment,
        networks: HashMap::new(),
        deploy,
    })
}

/// For a host-run (non-dockerized) local service that declares a `working_dir`,
/// write the undockerized environment as a `.env` file into that directory and
/// copy the service's secrets alongside it, so the service can be started by
/// hand from its own working directory. No-op when `working_dir` is unset.
pub fn write_working_dir(service: &ServiceResolvedSpec, spec: &EnvironmentResolvedSpec) -> anyhow::Result<()> {
    let Some(working_dir) = service.working_dir.as_deref() else {
        return Ok(());
    };

    let dir = Path::new(working_dir);
    fs::create_dir_all(dir).context(format!("Failed to create working_dir {:?}", dir))?;

    let mut env_vars = service.undockerized_environment_variables.clone();

    // Env-variable secrets are merged into `.env`; file secrets are written as
    // files relative to the working directory.
    for secret_option in &service.secrets {
        let Some(secret_spec) = spec.current_deployment.secrets.iter().find(|s| s.name == secret_option.name) else {
            eprintln!("Warning: Secret {} not found for service {}", secret_option.name, service.full_name);
            continue;
        };
        match &secret_option.mount {
            SecretMount::EnvVariable(var_name) => {
                env_vars.push(EnvVariable { name: var_name.clone(), value: secret_spec.value.clone() });
            }
            SecretMount::FilePath(mount_path) => {
                let rel_path = mount_path.trim_start_matches('/');
                let host_path = dir.join(rel_path);
                if let Some(parent) = host_path.parent() {
                    fs::create_dir_all(parent).context("Failed to create secret parent directory")?;
                }
                fs::write(&host_path, &secret_spec.value).context("Failed to write secret file")?;
            }
        }
    }

    let env_path = dir.join(".env");
    write_env_file(&env_path, &env_vars)
}

fn write_env_file(path: &Path, vars: &[EnvVariable]) -> anyhow::Result<()> {
    let content = vars.iter()
        .map(|v| format!("{}={}", v.name, v.value))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, content).context(format!("Failed to write env file {:?}", path))
}
