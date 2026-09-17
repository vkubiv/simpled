use crate::resolved_spec::{EnvironmentResolvedSpec, ServiceResolvedSpec};
use crate::spec;
use crate::spec::{EnvVariable, Healthcheck, SecretMount, ServiceCommand, ServiceType, ServiceVolumeType};
use anyhow::Context;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct DockerCompose {
    // Compose project name. Only set for the local run; `docker stack deploy`
    // takes the stack name on the command line and ignores this key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub services: HashMap<String, DockerService>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub networks: HashMap<String, DockerComposeNetwork>,
}

/// Compose project name for a local run.
///
/// Without an explicit name compose falls back to the directory holding the
/// file, which is always `local_env` - so every application on the machine
/// shares one project, and bringing one up removes the containers of the last
/// one (`--remove-orphans` treats them as orphans of its own project). Naming
/// the project after the application keeps them apart. Compose only accepts
/// `[a-z0-9][a-z0-9_-]*`, so anything else in the name folds into an underscore.
pub fn local_project_name(application_name: &str) -> String {
    let sanitized: String = application_name
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let sanitized = sanitized.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());

    if sanitized.is_empty() {
        "local".to_string()
    } else {
        format!("{}_local", sanitized)
    }
}

#[derive(Serialize, Clone)]
pub struct DockerService {
    pub image: String,
    // Only for the local compose file. `docker stack deploy` names containers
    // itself and ignores this key, so the stack file does not carry it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<ServiceCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<ServiceCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<Healthcheck>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expose: Vec<String>,
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
    // Start-order dependencies. `docker stack deploy` ignores this key, so it is
    // only filled in for compose files that are run with `docker compose up`;
    // Swarm ordering is handled by the phases of the generated deploy script.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub depends_on: HashMap<String, DependsOnCondition>,
}

#[derive(Serialize, Clone)]
pub struct DependsOnCondition {
    pub condition: String,
}

#[derive(Serialize, Clone, Default)]
pub struct DeployConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replicas: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<RestartPolicy>,
}

#[derive(Serialize, Clone)]
pub struct RestartPolicy {
    pub condition: String,
}

#[derive(Serialize)]
pub struct DockerComposeNetwork {
    pub external: bool,
    pub name: String,
}

#[derive(Serialize, Clone)]
pub struct ServiceNetwork {
    pub aliases: Vec<String>,
}

pub fn prepare_service(
    service: &ServiceResolvedSpec,
    spec: &EnvironmentResolvedSpec,
    output_dir: &Path,
) -> anyhow::Result<DockerService> {
    let svc_dir = output_dir.join(service.full_name.clone());
    fs::create_dir_all(&svc_dir).context("Failed to create service directory")?;

    // Generate .env files. The local compose file is read by `docker compose`,
    // which applies dotenv rules to `env_file`; the Swarm stack file is read by
    // `docker stack deploy`, whose legacy loader takes every value literally.
    let is_local = spec.env_type == spec::DeploymentEnvType::Local;
    let format = if is_local {
        EnvFileFormat::Compose
    } else {
        EnvFileFormat::Raw
    };
    let env_path = svc_dir.join(".env");
    write_env_file(&env_path, &service.environment_variables, format)?;

    // A host-run service with a `working_dir` gets its `.env` written into that
    // directory by `write_working_dir`, so skip the in-tree `undockerized.env`.
    if is_local && service.working_dir.is_none() {
        let undoc_env_path = svc_dir.join("undockerized.env");
        write_env_file(
            &undoc_env_path,
            &service.undockerized_environment_variables,
            EnvFileFormat::Raw,
        )?;
    }

    let mut volumes = Vec::new();
    let mut environment = HashMap::new();

    for volume in &service.volumes {
        match &volume.name {
            ServiceVolumeType::Named(name) => {
                volumes.push(format!("./volumes/{}:{}", name, volume.mount_path));
            }
            ServiceVolumeType::Path(from_path) => {
                volumes.push(format!("{}:{}", from_path, volume.mount_path));
            }
        }
    }

    // Configs
    for config_option in &service.configs {
        if let Some(config_spec) = spec
            .current_deployment
            .configs
            .iter()
            .find(|c| c.name == config_option.config_name)
        {
            let rel_path = config_option.mount_path.trim_start_matches('/');
            let host_path = svc_dir.join(rel_path);

            let is_file_mount = if config_spec.files.len() == 1 {
                let file = &config_spec.files[0];
                let mount_filename = Path::new(&config_option.mount_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
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
                volumes.push(format!(
                    "./{}/{}:{}",
                    service.full_name, rel_path_str, config_option.mount_path
                ));
            } else {
                fs::create_dir_all(&host_path).context("Failed to create config directory")?;
                for file in &config_spec.files {
                    let p = host_path.join(&file.name);
                    fs::write(&p, &file.content).context("Failed to write config file inside dir")?;
                }

                let rel_path_str = rel_path.replace("\\", "/");
                volumes.push(format!(
                    "./{}/{}:{}",
                    service.full_name, rel_path_str, config_option.mount_path
                ));
            }
        } else {
            eprintln!(
                "Warning: Config {} not found for service {}",
                config_option.config_name, service.full_name
            );
        }
    }

    // Secrets. A secret with an `aws` source has no value yet: `fetch-secrets.sh`
    // exports it as a shell variable and writes its file on the deploy target, so
    // the compose file only references it — through `${VAR}` interpolation, which
    // `docker stack deploy` resolves from its own environment — and the file mount
    // points at a path the fetch script will have populated.
    for secret_option in &service.secrets {
        if let Some(secret_spec) = spec
            .current_deployment
            .secrets
            .iter()
            .find(|s| s.name == secret_option.name)
        {
            match &secret_option.mount {
                SecretMount::EnvVariable(var_name) => {
                    let value = match secret_spec.literal() {
                        Some(literal) => literal.to_string(),
                        None => format!("${{{}}}", secret_spec.shell_var()),
                    };
                    environment.insert(var_name.clone(), value);
                }

                SecretMount::FilePath(mount_path) => {
                    let rel_path = mount_path.trim_start_matches('/');
                    let host_path = svc_dir.join(rel_path);

                    if let Some(parent) = host_path.parent() {
                        fs::create_dir_all(parent).context("Failed to create secret parent directory")?;
                    }
                    if let Some(literal) = secret_spec.literal() {
                        fs::write(&host_path, literal).context("Failed to write secret file")?;
                    }

                    let rel_path_str = rel_path.replace("\\", "/");
                    volumes.push(format!("./{}/{}:{}", service.full_name, rel_path_str, mount_path));
                }
            }
        } else {
            eprintln!(
                "Warning: Secret {} not found for service {}",
                secret_option.name, service.full_name
            );
        }
    }

    // Ports
    let ports = service
        .ports
        .iter()
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
    //
    // Replicas are only written for Swarm. A local compose file names every
    // container, and compose refuses `container_name` next to `replicas`; a job
    // is created by the deploy script rather than the stack file.
    let is_local = spec.env_type == spec::DeploymentEnvType::Local;
    let deploy = match service.service_type {
        ServiceType::Job => Some(DeployConfig {
            restart_policy: Some(RestartPolicy {
                condition: "none".to_string(),
            }),
            ..DeployConfig::default()
        }),
        _ if !is_local => Some(DeployConfig {
            replicas: Some(service.resources.replicas),
            ..DeployConfig::default()
        }),
        _ => None,
    };

    Ok(DockerService {
        image: service.image.clone(),
        container_name: is_local.then(|| service.full_name.clone()),
        entrypoint: service.entrypoint.clone(),
        command: service.command.clone(),
        healthcheck: service.healthcheck.clone(),
        ports,
        expose: service.expose.clone(),
        volumes,
        env_file: vec![format!("./{}/.env", service.full_name)],
        environment,
        networks: HashMap::new(),
        deploy,
        depends_on: HashMap::new(),
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
        let Some(secret_spec) = spec
            .current_deployment
            .secrets
            .iter()
            .find(|s| s.name == secret_option.name)
        else {
            eprintln!(
                "Warning: Secret {} not found for service {}",
                secret_option.name, service.full_name
            );
            continue;
        };
        // Host-run services exist for `local` only, where secrets with an `aws`
        // source are fetched during resolution, so a value is always available.
        let Some(value) = secret_spec.literal() else {
            eprintln!(
                "Warning: Secret {} is only fetched on the deploy target and cannot be written \
                to the working_dir of {}",
                secret_option.name, service.full_name
            );
            continue;
        };
        match &secret_option.mount {
            SecretMount::EnvVariable(var_name) => {
                env_vars.push(EnvVariable {
                    name: var_name.clone(),
                    value: value.to_string(),
                });
            }
            SecretMount::FilePath(mount_path) => {
                let rel_path = mount_path.trim_start_matches('/');
                let host_path = dir.join(rel_path);
                if let Some(parent) = host_path.parent() {
                    fs::create_dir_all(parent).context("Failed to create secret parent directory")?;
                }
                fs::write(&host_path, value).context("Failed to write secret file")?;
            }
        }
    }

    let env_path = dir.join(".env");
    // Read by whatever dotenv loader the developer's own process uses, so the
    // plainest form is the most portable one.
    write_env_file(&env_path, &env_vars, EnvFileFormat::Raw)
}

/// How the reader of an env file interprets a value.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EnvFileFormat {
    /// `docker run --env-file`, `docker stack deploy` and most dotenv loaders:
    /// everything after `=` is the value, byte for byte, up to the end of the
    /// line. Nothing can be escaped, so a value cannot contain a line break.
    Raw,
    /// `docker compose` (compose-go): an unquoted value has `${VAR}` expanded and
    /// ` #` starts a comment, and quotes are interpreted. A single-quoted value
    /// is taken literally, with `\'` for a quote inside it.
    Compose,
}

/// One `NAME=value` line in `format`, or an error when the value cannot be
/// represented in it at all.
fn env_file_line(var: &EnvVariable, format: EnvFileFormat) -> anyhow::Result<String> {
    match format {
        EnvFileFormat::Raw => {
            if var.value.contains(['\n', '\r']) {
                anyhow::bail!(
                    "Environment variable {} contains a line break, which an env file cannot carry. \
                     Mount the value as a secret file instead.",
                    var.name
                );
            }
            Ok(format!("{}={}", var.name, var.value))
        }
        EnvFileFormat::Compose => Ok(format!("{}='{}'", var.name, var.value.replace('\'', "\\'"))),
    }
}

fn write_env_file(path: &Path, vars: &[EnvVariable], format: EnvFileFormat) -> anyhow::Result<()> {
    let content = vars
        .iter()
        .map(|v| env_file_line(v, format))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("\n");
    fs::write(path, content).context(format!("Failed to write env file {:?}", path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var(name: &str, value: &str) -> EnvVariable {
        EnvVariable {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    /// `docker compose` expands `$`, strips ` #` comments and interprets quotes
    /// in an unquoted env_file value; a single-quoted value is left alone.
    #[test]
    fn compose_env_values_are_single_quoted() {
        assert_eq!(
            env_file_line(&var("PASSWORD", "pa$$ #word"), EnvFileFormat::Compose).unwrap(),
            "PASSWORD='pa$$ #word'"
        );
        assert_eq!(
            env_file_line(&var("QUOTE", "it's"), EnvFileFormat::Compose).unwrap(),
            "QUOTE='it\\'s'"
        );
    }

    /// `docker run --env-file` and `docker stack deploy` take the value literally,
    /// so quoting there would put the quotes into the container.
    #[test]
    fn raw_env_values_are_written_as_is() {
        assert_eq!(
            env_file_line(&var("PASSWORD", "pa$$ #word 'q'"), EnvFileFormat::Raw).unwrap(),
            "PASSWORD=pa$$ #word 'q'"
        );
    }

    #[test]
    fn a_raw_env_value_cannot_contain_a_line_break() {
        let err = env_file_line(&var("PEM", "a\nb"), EnvFileFormat::Raw).unwrap_err();
        assert!(err.to_string().contains("line break"), "{}", err);
    }

    #[test]
    fn project_name_suffixes_the_application_name() {
        assert_eq!(local_project_name("room_scaner_backend"), "room_scaner_backend_local");
        assert_eq!(local_project_name("shop-api"), "shop-api_local");
    }

    #[test]
    fn project_name_is_a_valid_compose_project() {
        // Compose rejects anything outside [a-z0-9][a-z0-9_-]*, so an application
        // name with capitals, spaces or dots must not reach it unchanged.
        assert_eq!(local_project_name("Room Scaner"), "room_scaner_local");
        assert_eq!(local_project_name("acme.shop"), "acme_shop_local");
        assert_eq!(local_project_name("_leading"), "leading_local");
        assert_eq!(local_project_name("***"), "local");
    }

    mod prepare {
        use super::*;
        use crate::resolved_spec::{ConfigResolvedFile, ConfigResolvedSpec, SecretResolvedSpec, SecretResolvedValue};
        use crate::spec::{AwsSecretRef, DeploymentEnvType, ServiceConfigOption, ServiceSecret, ServiceType};
        use crate::test_support::{resolved_deployment, resolved_env, resolved_service};

        fn config(name: &str, files: &[(&str, &str)]) -> ConfigResolvedSpec {
            ConfigResolvedSpec {
                name: name.to_string(),
                files: files
                    .iter()
                    .map(|(n, c)| ConfigResolvedFile {
                        name: n.to_string(),
                        content: c.as_bytes().to_vec(),
                    })
                    .collect(),
            }
        }

        fn swarm() -> DeploymentEnvType {
            DeploymentEnvType::Docker(crate::spec::DockerSpecificSpec {
                ingress_type: crate::spec::DockerIngressType::Nginx,
                swarm_mode: true,
            })
        }

        #[test]
        fn a_single_file_config_is_mounted_as_a_file_and_a_group_as_a_directory() {
            let mut api = resolved_service("api", ServiceType::Public, &[]);
            api.configs = vec![
                ServiceConfigOption {
                    config_name: "shop-settings".to_string(),
                    mount_path: "/etc/app/settings.json".to_string(),
                },
                ServiceConfigOption {
                    config_name: "shop-data".to_string(),
                    mount_path: "/data".to_string(),
                },
            ];
            let mut deployment = resolved_deployment(vec![]);
            deployment.configs = vec![
                config("shop-settings", &[("settings.json", "{}")]),
                config("shop-data", &[("a.json", "1"), ("b.json", "2")]),
            ];
            let spec = resolved_env(swarm(), deployment);

            let out = tempfile::tempdir().unwrap();
            let service = prepare_service(&api, &spec, out.path()).unwrap();

            // The mount path names the file itself, so it is a file mount.
            assert!(
                service
                    .volumes
                    .contains(&"./api/etc/app/settings.json:/etc/app/settings.json".to_string()),
                "{:?}",
                service.volumes
            );
            assert_eq!(
                fs::read_to_string(out.path().join("api/etc/app/settings.json")).unwrap(),
                "{}"
            );
            // Several files: the directory is mounted whole.
            assert!(
                service.volumes.contains(&"./api/data:/data".to_string()),
                "{:?}",
                service.volumes
            );
            assert_eq!(fs::read_to_string(out.path().join("api/data/b.json")).unwrap(), "2");
            assert_eq!(service.env_file, vec!["./api/.env"]);
        }

        #[test]
        fn literal_secrets_are_written_and_deferred_ones_referenced() {
            let mut api = resolved_service("api", ServiceType::Public, &[]);
            api.secrets = vec![
                ServiceSecret {
                    name: "shop-api_key".to_string(),
                    mount: SecretMount::EnvVariable("API_KEY".to_string()),
                },
                ServiceSecret {
                    name: "shop-db_password".to_string(),
                    mount: SecretMount::EnvVariable("DB_PASSWORD".to_string()),
                },
                ServiceSecret {
                    name: "shop-tls_key".to_string(),
                    mount: SecretMount::FilePath("/run/secrets/tls.key".to_string()),
                },
                ServiceSecret {
                    name: "shop-tls_cert".to_string(),
                    mount: SecretMount::FilePath("/run/secrets/tls.crt".to_string()),
                },
            ];
            let mut deployment = resolved_deployment(vec![]);
            let deferred = |name: &str| SecretResolvedSpec {
                name: name.to_string(),
                value: SecretResolvedValue::Deferred(AwsSecretRef {
                    secret_id: "prod/x".to_string(),
                    jq: None,
                }),
            };
            let literal = |name: &str, value: &str| SecretResolvedSpec {
                name: name.to_string(),
                value: SecretResolvedValue::Literal(value.to_string()),
            };
            deployment.secrets = vec![
                literal("shop-api_key", "k3y"),
                deferred("shop-db_password"),
                deferred("shop-tls_key"),
                literal("shop-tls_cert", "pem"),
            ];
            let spec = resolved_env(swarm(), deployment);

            let out = tempfile::tempdir().unwrap();
            let service = prepare_service(&api, &spec, out.path()).unwrap();

            assert_eq!(service.environment["API_KEY"], "k3y");
            assert_eq!(service.environment["DB_PASSWORD"], "${SIMPLED_SECRET_SHOP_DB_PASSWORD}");
            // Both file secrets are mounted; only the literal one exists yet.
            assert!(service
                .volumes
                .contains(&"./api/run/secrets/tls.key:/run/secrets/tls.key".to_string()));
            assert!(service
                .volumes
                .contains(&"./api/run/secrets/tls.crt:/run/secrets/tls.crt".to_string()));
            assert!(!out.path().join("api/run/secrets/tls.key").exists());
            assert_eq!(
                fs::read_to_string(out.path().join("api/run/secrets/tls.crt")).unwrap(),
                "pem"
            );
        }

        #[test]
        fn a_job_disables_restarts_and_a_service_carries_its_replicas() {
            let mut api = resolved_service("api", ServiceType::Public, &[]);
            api.resources.replicas = 3;
            let job = resolved_service("migrate", ServiceType::Job, &[]);
            let spec = resolved_env(swarm(), resolved_deployment(vec![]));
            let out = tempfile::tempdir().unwrap();

            let api = prepare_service(&api, &spec, out.path()).unwrap();
            assert_eq!(api.deploy.as_ref().unwrap().replicas, Some(3));
            assert!(api.deploy.as_ref().unwrap().restart_policy.is_none());
            assert!(api.container_name.is_none());

            let job = prepare_service(&job, &spec, out.path()).unwrap();
            assert_eq!(
                job.deploy.as_ref().unwrap().restart_policy.as_ref().unwrap().condition,
                "none"
            );
            assert!(job.deploy.as_ref().unwrap().replicas.is_none());
        }

        #[test]
        fn a_local_service_is_named_and_gets_its_undockerized_file() {
            let mut api = resolved_service("api", ServiceType::Public, &[]);
            api.undockerized_environment_variables = vec![EnvVariable {
                name: "DB_HOST".to_string(),
                value: "localhost".to_string(),
            }];
            let spec = resolved_env(DeploymentEnvType::Local, resolved_deployment(vec![]));
            let out = tempfile::tempdir().unwrap();

            let service = prepare_service(&api, &spec, out.path()).unwrap();
            assert_eq!(service.container_name.as_deref(), Some("api"));
            assert!(service.deploy.is_none());
            assert_eq!(
                fs::read_to_string(out.path().join("api/undockerized.env")).unwrap(),
                "DB_HOST=localhost"
            );
        }

        #[test]
        fn a_working_dir_receives_the_env_and_the_secrets_it_can_have() {
            let root = tempfile::tempdir().unwrap();
            let mut worker = resolved_service("worker", ServiceType::Internal, &[]);
            worker.working_dir = Some(root.path().join("worker-src").to_string_lossy().into_owned());
            worker.undockerized_environment_variables = vec![EnvVariable {
                name: "DB_HOST".to_string(),
                value: "localhost".to_string(),
            }];
            worker.secrets = vec![
                ServiceSecret {
                    name: "shop-db_password".to_string(),
                    mount: SecretMount::EnvVariable("DB_PASSWORD".to_string()),
                },
                ServiceSecret {
                    name: "shop-tls_key".to_string(),
                    mount: SecretMount::FilePath("/run/secrets/tls.key".to_string()),
                },
                ServiceSecret {
                    name: "shop-remote".to_string(),
                    mount: SecretMount::EnvVariable("REMOTE".to_string()),
                },
            ];
            let mut deployment = resolved_deployment(vec![]);
            deployment.secrets = vec![
                SecretResolvedSpec {
                    name: "shop-db_password".to_string(),
                    value: SecretResolvedValue::Literal("pw".to_string()),
                },
                SecretResolvedSpec {
                    name: "shop-tls_key".to_string(),
                    value: SecretResolvedValue::Literal("pem".to_string()),
                },
                SecretResolvedSpec {
                    name: "shop-remote".to_string(),
                    value: SecretResolvedValue::Deferred(AwsSecretRef {
                        secret_id: "x".to_string(),
                        jq: None,
                    }),
                },
            ];
            let spec = resolved_env(DeploymentEnvType::Local, deployment);

            write_working_dir(&worker, &spec).unwrap();

            let env = fs::read_to_string(root.path().join("worker-src/.env")).unwrap();
            assert!(env.contains("DB_HOST=localhost"), "{env}");
            assert!(env.contains("DB_PASSWORD=pw"), "{env}");
            // A secret without a value here is skipped rather than written blank.
            assert!(!env.contains("REMOTE"), "{env}");
            assert_eq!(
                fs::read_to_string(root.path().join("worker-src/run/secrets/tls.key")).unwrap(),
                "pem"
            );

            // No working_dir, nothing written.
            let plain = resolved_service("api", ServiceType::Public, &[]);
            write_working_dir(&plain, &spec).unwrap();
        }
    }
}
