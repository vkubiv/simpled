use crate::spec::*;
use crate::spec_yaml::*;
use crate::{env_loader, spec};
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const DEFAULT_MEMORY: &str = "128Mi";
const DEFAULT_CPU: &str = "100m";

pub fn convert_env_spec(yaml: DeploymentEnvironmentSpecYaml, root: &Path, selected_deployment: Option<&str>) -> Result<DeploymentEnvironmentSpec> {
    let env_type_yaml = yaml.env_type
        .ok_or_else(|| anyhow!("'type' field is required in env spec"))?;
    let swarm_mode_opt = yaml.swarm_mode;
    let gateway_yaml = match (yaml.gateway, yaml.ingress) {
        (Some(g), _) => g,
        (None, Some(i)) => {
            eprintln!("Warning: 'ingress' in env spec is deprecated; rename it to 'gateway'");
            i
        }
        (None, None) => return Err(anyhow!("'gateway' field is required in env spec")),
    };
    let ingress_type_str = gateway_yaml.ingress_type.clone();
    let ingress = convert_ingress(gateway_yaml, &env_type_yaml)?;
    let registry = yaml.registry.unwrap_or_default();

    let mut deployments = Vec::new();
    for (name, dep) in &yaml.deployments {
        deployments.push(convert_deployment(name.clone(), dep, root, &env_type_yaml)?);
    }

    let env_type = match env_type_yaml {
        DeploymentEnvTypeYaml::K8S => {
            if swarm_mode_opt.is_some() {
                return Err(anyhow!("swarm_mode cannot be set for K8S environment"));
            }
            if ingress_type_str.is_some() {
                return Err(anyhow!("ingress_type cannot be set for K8S environment"));
            }
            if yaml.deployments.values().any(|d| d.secrets_folder.is_some()) {
                return Err(anyhow!("secrets_folder cannot be set for K8S environment"));
            }
            if any_service_has_working_dir(&yaml.deployments) {
                return Err(anyhow!("working_dir cannot be set for K8S environment"));
            }
            DeploymentEnvType::K8S
        },
        DeploymentEnvTypeYaml::Docker => {
            let swarm_mode = swarm_mode_opt.unwrap_or(false);
            let ingress_type = match ingress_type_str.as_deref() {
                Some("nginx") => DockerIngressType::Nginx,
                Some("traefik") | None => DockerIngressType::Traefik,
                Some(other) => return Err(anyhow!("Unknown ingress type: {}", other)),
            };
            if yaml.deployments.values().any(|d| d.secrets_folder.is_some()) {
                return Err(anyhow!("secrets_folder cannot be set for Docker environment"));
            }
            if any_service_has_working_dir(&yaml.deployments) {
                return Err(anyhow!("working_dir cannot be set for Docker environment"));
            }
            DeploymentEnvType::Docker(DockerSpecificSpec {
                swarm_mode,
                ingress_type,
            })
        },
        DeploymentEnvTypeYaml::Local => {
            if swarm_mode_opt.is_some() {
                return Err(anyhow!("swarm_mode cannot be set for Local environment"));
            }
            if ingress_type_str.is_some() {
                return Err(anyhow!("ingress_type cannot be set for Local environment"));
            }
            if !registry.is_empty() {
                return Err(anyhow!("registry must be empty for Local environment"));
            }
            if deployments.is_empty() {
                return Err(anyhow!("For Local environment at least one deployment must be specified"));
            }
            // Multiple deployments are allowed, but only one runs at a time. When more
            // than one is defined the caller must pick which with --deployment.
            if deployments.len() > 1 && selected_deployment.is_none() {
                return Err(anyhow!(
                    "For Local environment with multiple deployments, specify which one with --deployment"
                ));
            }

            // Port uniqueness is checked per deployment: different deployments may reuse
            // the same external ports since only one runs at a time.
            for dep in &deployments {
                let mut ports_seen = HashSet::new();
                if let Some(services) = &dep.services {
                    for (svc_name, svc_spec) in services {
                        if svc_spec.ports.is_empty() {
                            return Err(anyhow!("In Local environment, service {} must have at least one port", svc_name));
                        }
                        for port in &svc_spec.ports {
                            if !ports_seen.insert(port.external) {
                                return Err(anyhow!("Duplicate external port {} in deployment {}", port.external, dep.name));
                            }
                        }
                    }
                }
            }

            DeploymentEnvType::Local
        },
    };

    Ok(DeploymentEnvironmentSpec {
        env_type,
        ingress,
        registry,
        deployments,
    })
}

fn any_service_has_working_dir(deployments: &HashMap<String, DeploymentSpecYaml>) -> bool {
    deployments.values().any(|d| {
        d.services
            .as_ref()
            .map_or(false, |svcs| svcs.values().any(|s| s.working_dir.is_some()))
    })
}

fn convert_ingress(yaml: IngressSpecYaml, env_type: &DeploymentEnvTypeYaml) -> Result<IngressSpec> {
    let mut hosts = Vec::new();
    for (name, host) in yaml.hosts {
        match host {
            HostSpecYaml::Single(s) => hosts.push(HostSpec { name, domain_names: vec![s] }),
            HostSpecYaml::Multiple(v) => hosts.push(HostSpec { name, domain_names: v }),
        }
    }

    let tls = match (yaml.tls, env_type) {
        (None, DeploymentEnvTypeYaml::Local) => None,
        (None, _) => return Err(anyhow!("Ingress TLS configuration is required for non-local environments. If you want to disable TLS explicitly set 'disable: true' in tls section")),
        (Some(t), _) => {
            // TODO: if this local env and tls is enabled rise error.
            if t.disable == Some(true) {
                None
            } else {
                let letsencrypt = t.letsencrypt.map(|le| LetsEncryptSpec {
                    server: le.server,
                    email: le.email,
                });
                Some(IngressTlsSpec {
                    secret: t.secret,
                    letsencrypt,
                })
            }
        }
    };

    Ok(IngressSpec {
        name: yaml.name,
        hosts,
        tls,
    })
}

fn convert_env_variables(yaml: &Option<DeploymentEnvVariablesYaml>, root: &Path) -> Result<Vec<spec::EnvVariable>> {
    match yaml {
        Some(DeploymentEnvVariablesYaml::FromEnvFile(env_file)) => env_loader::load_env_file(env_file),
        Some(DeploymentEnvVariablesYaml::FromList(entries)) => {
            entries.iter()
                .map(|entry| convert_env_entry(entry, root))
                .collect::<Result<Vec<_>>>()
        },
        None => Ok(vec![]),
    }
}

fn convert_env_entry(entry: &EnvVariableEntryYaml, root: &Path) -> Result<spec::EnvVariable> {
    match entry {
        EnvVariableEntryYaml::Inline(s) => env_loader::parse_env_variable(s),
        EnvVariableEntryYaml::FromFile(map) => {
            if map.len() != 1 {
                return Err(anyhow!(
                    "A file-backed environment entry must define exactly one variable, got {}",
                    map.len()
                ));
            }
            let (name, source) = map.iter().next().unwrap();
            let file_path = root.join(&source.file);
            let value = fs::read_to_string(&file_path)
                .with_context(|| format!("Failed to read value for env variable '{}' from {:?}", name, file_path))?;
            // Files commonly end with a trailing newline that is not part of the value.
            let value = value.trim_end_matches(|c| c == '\n' || c == '\r').to_string();
            Ok(spec::EnvVariable {
                name: name.clone(),
                value,
            })
        }
    }
}

fn convert_deployment(name: String, yaml: &DeploymentSpecYaml, root: &Path, env_type: &DeploymentEnvTypeYaml) -> Result<DeploymentSpec> {
    let secrets_folder = yaml.secrets_folder.as_deref().map(|s| root.join(s));
    let application = convert_deployment_app(&yaml.application)?;
    let environment = convert_env_variables(&yaml.environment, root)?;
    let mut undockerized_environment = convert_env_variables(&yaml.undockerized_environment, root)?;

    // For local runs, a `.env.local` file in the project root overrides
    // `undockerized_environment` variables, letting each developer tweak the
    // values used by host-run (non-dockerized) services without editing the
    // committed env files.
    if matches!(env_type, DeploymentEnvTypeYaml::Local) {
        let env_local_path = root.join(".env.local");
        if env_local_path.exists() {
            let overrides = env_loader::load_env_file(&env_local_path)?;
            for var in overrides {
                if let Some(existing) = undockerized_environment.iter_mut().find(|v| v.name == var.name) {
                    existing.value = var.value;
                } else {
                    undockerized_environment.push(var);
                }
            }
        }
    }

    let configs = if let Some(conf) = &yaml.configs {
        let mut specs = Vec::new();
        for (k, v) in conf {
            let path = root.join(Path::new(&v));
            let files = if path.is_dir() {
                let mut files = Vec::new();
                for entry in fs::read_dir(path).with_context(|| format!("Failed to read config directory {}", v))? {
                    let entry = entry?;
                    let p = entry.path();
                    if p.is_file() {
                        files.push(p.to_string_lossy().to_string());
                    }
                }
                files
            } else {
                return Err(anyhow!("Config path {} is not a directory", v));
            };
            specs.push(ConfigSpec { name: k.clone(), files });
        }
        specs
    } else {
        vec![]
    };

    let secrets = if let Some(sec) = &yaml.secrets {
        let mut list = Vec::new();
        for (k, v) in sec {
            match v {
                DeploymentSecretSpecExYaml::Detailed(v) => {
                    if v.env.is_some() && v.file.is_some() {
                        return Err(anyhow!("Secret {} cannot have both env and file sources", k));
                    }
                    let source = if let Some(env) = &v.env {
                        DeploymentSecretSource::EnvVariable(env.clone())
                    } else if let Some(file) = &v.file {
                        DeploymentSecretSource::FilePath(file.clone())
                    } else {
                        return Err(anyhow!("Secret {} must have either env or file source", k));
                    };
                    list.push(DeploymentSecretSpec {
                        secret_name: k.clone(),
                        source,
                    });
                }
                DeploymentSecretSpecExYaml::Local(opt_value) => {
                    let resolved = match opt_value.as_deref() {
                        Some(v) if !v.is_empty() => v.to_string(),
                        _ => {
                            let folder = secrets_folder.as_ref().ok_or_else(|| {
                                anyhow!("Secret '{}' has no value but secrets_folder is not configured", k)
                            })?;
                            let secret_path = folder.join(k);
                            fs::read_to_string(&secret_path)
                                .context(format!("Failed to read secret '{}' from {:?}", k, secret_path))?
                        }
                    };
                    list.push(DeploymentSecretSpec {
                        secret_name: k.clone(),
                        source: DeploymentSecretSource::Embedded(resolved),
                    });
                }
            }
        }
        list
    } else {
        vec![]
    };

    let defaults = if let Some(def) = &yaml.defaults {
        convert_defaults(def)?
    } else {
        ResourcesSpec {
            replicas: 1,
            requests: ResourceLimits { memory: DEFAULT_MEMORY.into(), cpu: DEFAULT_CPU.into() },
            limits: ResourceLimits { memory: DEFAULT_MEMORY.into(), cpu: DEFAULT_CPU.into() },
        }
    };

    let services = if let Some(svcs) = &yaml.services {
        let mut map = HashMap::new();
        for (k, v) in svcs {
            map.insert(k.clone(), convert_deployment_service(v, &defaults)?);
        }
        Some(map)
    } else {
        None
    };

    Ok(DeploymentSpec {
        primary_host: yaml.primary_host.clone(),
        name,
        application,
        environment,
        undockerized_environment,
        configs,
        secrets,
        defaults,
        services,
    })
}

fn convert_deployment_app(yaml: &DeploymentAppSpecYaml) -> Result<DeploymentAppSpec> {
    let version = if let Some(v) = &yaml.version {
        Some(semver::VersionReq::parse(v)?)
    } else {
        None
    };

    Ok(DeploymentAppSpec {
        name: yaml.name.clone(),
        version,
        extra: yaml.extra.clone().unwrap_or_default(),
    })
}

fn convert_defaults(yaml: &DefaultsSpecYaml) -> Result<ResourcesSpec> {
    let replicas = yaml.replicas.unwrap_or(1);
    let (requests, limits) = if let Some(res) = &yaml.resources {
        (
            convert_limits(res.requests.as_ref()),
            convert_limits(res.limits.as_ref()),
        )
    } else {
        (
            ResourceLimits { memory: DEFAULT_MEMORY.into(), cpu: DEFAULT_CPU.into() },
            ResourceLimits { memory: DEFAULT_MEMORY.into(), cpu: DEFAULT_CPU.into() },
        )
    };

    Ok(ResourcesSpec { replicas, requests, limits })
}

fn convert_limits(yaml: Option<&ResourceLimitsYaml>) -> ResourceLimits {
    if let Some(l) = yaml {
        ResourceLimits {
            memory: l.memory.clone().unwrap_or_else(|| DEFAULT_MEMORY.into()),
            cpu: l.cpu.clone().unwrap_or_else(|| DEFAULT_CPU.into()),
        }
    } else {
        ResourceLimits { memory: DEFAULT_MEMORY.into(), cpu: DEFAULT_CPU.into() }
    }
}

fn convert_deployment_service(yaml: &DeploymentServiceSpecYaml, defaults: &ResourcesSpec) -> Result<DeploymentServiceSpec> {
    let mut prefixes = if let Some(p) = &yaml.prefixes {
        p.iter().map(|(k, v)| Prefix {
            prefix: k.clone(),
            strip: v.strip.unwrap_or(false),
        }).collect()
    } else {
        Vec::new()
    };

    if let Some(prefix) = &yaml.prefix {
        prefixes.push(Prefix { prefix: prefix.clone(), strip: yaml.strip_prefix.unwrap_or(true) });
    }

    let resources = if let Some(res) = &yaml.resources {
        ResourcesSpec {
            replicas: yaml.replicas.unwrap_or(defaults.replicas),
            requests: convert_limits(res.requests.as_ref()),
            limits: convert_limits(res.limits.as_ref()),
        }
    } else {
        ResourcesSpec {
            replicas: yaml.replicas.unwrap_or(defaults.replicas),
            requests: ResourceLimits {
                memory: defaults.requests.memory.clone(),
                cpu: defaults.requests.cpu.clone(),
            },
            limits: ResourceLimits {
                memory: defaults.limits.memory.clone(),
                cpu: defaults.limits.cpu.clone(),
            },
        }
    };

    let ports = super::parse_ports(&yaml.ports)?;

    Ok(DeploymentServiceSpec {
        variant: yaml.variant.clone(),
        host: yaml.host.clone(),
        prefixes,
        resources,
        ports,
        working_dir: yaml.working_dir.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn local_env_yaml() -> DeploymentEnvironmentSpecYaml {
        let raw = r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  app_local:
    primary_host: web
    application:
      name: app
    undockerized_environment:
      - DB_HOST=docker-db
      - REDIS_HOST=docker-redis
    services:
      web:
        host: web
        prefix: /
        ports:
          - "8080:80"
"#;
        serde_yaml::from_str(raw).unwrap()
    }

    fn undockerized(spec: &DeploymentEnvironmentSpec) -> &[EnvVariable] {
        &spec.deployments[0].undockerized_environment
    }

    #[test]
    fn env_local_overrides_and_appends_undockerized_vars() {
        let root = tempfile::tempdir().unwrap();
        let mut f = fs::File::create(root.path().join(".env.local")).unwrap();
        // DB_HOST overrides an existing var; EXTRA is appended.
        writeln!(f, "DB_HOST=localhost").unwrap();
        writeln!(f, "EXTRA=value").unwrap();

        let spec = convert_env_spec(local_env_yaml(), root.path(), None).unwrap();
        let vars = undockerized(&spec);

        let db = vars.iter().find(|v| v.name == "DB_HOST").unwrap();
        assert_eq!(db.value, "localhost");
        // unchanged var stays as defined in the spec
        assert_eq!(vars.iter().find(|v| v.name == "REDIS_HOST").unwrap().value, "docker-redis");
        assert_eq!(vars.iter().find(|v| v.name == "EXTRA").unwrap().value, "value");
    }

    #[test]
    fn reads_env_variable_value_from_file() {
        let root = tempfile::tempdir().unwrap();
        let mut f = fs::File::create(root.path().join("db_url")).unwrap();
        // trailing newline must be stripped
        writeln!(f, "postgres://localhost/main").unwrap();

        let raw = r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  app_local:
    primary_host: web
    application:
      name: app
    undockerized_environment:
      - PLAIN=value
      - MAIN_SERVICE_DB:
          file: db_url
    services:
      web:
        host: web
        prefix: /
        ports:
          - "8080:80"
"#;
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(raw).unwrap();
        let spec = convert_env_spec(yaml, root.path(), None).unwrap();
        let vars = undockerized(&spec);

        assert_eq!(vars.iter().find(|v| v.name == "PLAIN").unwrap().value, "value");
        assert_eq!(
            vars.iter().find(|v| v.name == "MAIN_SERVICE_DB").unwrap().value,
            "postgres://localhost/main"
        );
    }

    #[test]
    fn missing_env_local_leaves_undockerized_vars_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let spec = convert_env_spec(local_env_yaml(), root.path(), None).unwrap();
        let vars = undockerized(&spec);
        assert_eq!(vars.len(), 2);
        assert_eq!(vars.iter().find(|v| v.name == "DB_HOST").unwrap().value, "docker-db");
    }
}
