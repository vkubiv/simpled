use crate::resolved_spec::*;
use crate::spec::{EnvVariable, SecretMount};
use anyhow::{Result, Context, anyhow};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::collections::HashMap;
use serde::Serialize;

#[derive(Serialize)]
struct DockerCompose {
    services: HashMap<String, DockerService>,
}

#[derive(Serialize)]
struct DockerService {
    image: String,
    container_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    env_file: Vec<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    environment: HashMap<String, String>,
}

pub fn run(spec: &EnvironmentResolvedSpec) -> Result<()> {
    let output_dir = Path::new("local_env");
    fs::create_dir_all(output_dir).context("Failed to create local_env directory")?;

    println!("Starting services for deployment: {}", spec.current_deployment.name);

    let mut services_map = HashMap::new();

    for service in &spec.current_deployment.services {
        let docker_service = prepare_service(service, spec, output_dir)?;
        services_map.insert(service.full_name.clone(), docker_service);
    }

    let compose = DockerCompose {
        services: services_map,
    };

    let compose_path = output_dir.join("docker-compose.yaml");
    let yaml = serde_yaml::to_string(&compose)?;
    fs::write(&compose_path, yaml)?;

    println!("Generated docker-compose.yaml at {:?}", compose_path);
    println!("Running docker compose up...");

    // Run docker compose
    let status = Command::new("docker")
        .current_dir(output_dir)
        .args(&["compose", "up", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;

    if !status.success() {
        return Err(anyhow!("docker compose failed"));
    }
    
    Ok(())
}

fn prepare_service(service: &ServiceResolvedSpec, spec: &EnvironmentResolvedSpec, output_dir: &Path) -> Result<DockerService> {
    let svc_dir = output_dir.join(service.full_name.clone());
    fs::create_dir_all(&svc_dir).context("Failed to create service directory")?;

    // Generate .env files
    let env_path = svc_dir.join(".env".to_string());
    write_env_file(&env_path, &service.environment_variables)?;

    let undoc_env_path = svc_dir.join("undockerized.env".to_string());
    write_env_file(&undoc_env_path, &service.undockerized_environment_variables)?;

    let mut volumes = Vec::new();
    let mut environment = HashMap::new();

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

    Ok(DockerService {
        image: service.image.clone(),
        container_name: service.full_name.clone(),
        ports,
        volumes,
        env_file: vec![format!("./{}/.env", service.full_name)],
        environment,
    })
}

fn write_env_file(path: &Path, vars: &[EnvVariable]) -> Result<()> {
    let content = vars.iter()
        .map(|v| format!("{}={}", v.name, v.value))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(path, content).context(format!("Failed to write env file {:?}", path))
}
