use crate::spec::*;
use crate::spec_yaml::*;
use crate::env_loader::parse_env_string;
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use crate::{env_loader, spec};
use std::fs;
use std::path::Path;

pub fn convert_app_spec(yaml: AppSpecYaml, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<AppSpec> {
    let version = semver::Version::parse(&yaml.version)
        .context("Failed to parse app version")?;

    let environment = if let Some(env) = yaml.environment {
        convert_environment(env)?
    } else {
        AppEnvironment {
            external: vec![],
            optional: vec![],
            relative: vec![],
            internal: vec![],
        }
    };

    let secrets = if let Some(sec) = yaml.secrets {
        convert_secrets(sec)?
    } else {
        vec![]
    };

    let configs = if let Some(conf) = yaml.configs {
        conf.into_iter().map(|(k, v)| ConfigSpec {
            name: k,
            files: v,
        }).collect()
    } else {
        vec![]
    };

    let app_services = if let Some(services) = yaml.app_services {
        convert_services(services)?
    } else {
        vec![]
    };

    let mut combined_extra_services = HashMap::new();

    if let Some(env) = env_spec {
        if let Some(deployment) = env.deployments.iter().find(|d| d.application.name == yaml.name) {
            for extra_file in &deployment.application.extra {
                let content = fs::read_to_string(extra_file)
                    .with_context(|| format!("Failed to read extra spec file {}", extra_file))?;
                let extra_yaml: ExtraAppSpecYaml = serde_yaml::from_str(&content)
                    .with_context(|| format!("Failed to parse extra spec file {}", extra_file))?;

                if let Some(services) = extra_yaml.extra_services {
                    combined_extra_services.extend(services);
                }
            }
        }
    }

    if let Some(services) = yaml.extra_services {
        combined_extra_services.extend(services);
    }

    let extra_services = convert_services(combined_extra_services)?;

    Ok(AppSpec {
        name: yaml.name,
        version,
        environment,
        app_services,
        extra_services,
        configs,
        secrets,
    })
}

fn convert_environment(yaml: AppEnvironmentYaml) -> Result<AppEnvironment> {
    let external = yaml.external.unwrap_or_default().into_iter().map(|s| {
        let desc = parse_env_string(&s)?;
        Ok(ExternalEnvVariable {
            name: desc.name,
            default: desc.default,
        })
    }).collect::<Result<Vec<_>>>()?;

    // TODO: check if optional variable is not defined in external
    let optional = yaml.optional.unwrap_or_default().into_iter().map(|s| {
        let desc = parse_env_string(&s)?;
        if desc.default.is_some() {
            return Err(anyhow!("Optional env variable {} cannot have a default value", desc.name));
        }
        Ok(OptionalEnvVariable {
            name: desc.name,
        })
    }).collect::<Result<Vec<_>>>()?;

    let relative = yaml.relative.unwrap_or_default().into_iter().map(|s| {
        let desc = parse_env_string(&s)
            .map_err(|_| anyhow!("Invalid relative env variable format: {}", s))?;

        let val = desc.default.ok_or_else(|| anyhow!("Invalid relative env variable format (no value): {}", s))?;

        if !val.starts_with('/') {
            return Err(anyhow!("Relative URL for {} must start with /", desc.name));
        }

        Ok(RelativeEnvVariable {
            name: desc.name,
            relative_value: val,
        })
    }).collect::<Result<Vec<_>>>()?;

    let internal = yaml.internal.unwrap_or_default().into_iter().map(|s| {
        let desc = parse_env_string(&s)
            .map_err(|_| anyhow!("Invalid internal env variable format: {}", s))?;

        if let Some(val) = desc.default {
             Ok(InternalEnvVariable {
                 name: desc.name,
                 value: val,
             })
        } else {
             Err(anyhow!("Internal env variable {} must have a value", desc.name))
        }
    }).collect::<Result<Vec<_>>>()?;

    Ok(AppEnvironment {
        external,
        optional,
        relative,
        internal,
    })
}

fn convert_secrets(yaml: AppSecretsYaml) -> Result<Vec<AppSecretOption>> {
    match yaml {
        AppSecretsYaml::Simple(list) => {
            Ok(list.into_iter().map(|s| AppSecretOption { secret_name: s }).collect())
        }
        AppSecretsYaml::Detailed(map) => {
            Ok(map.into_iter().map(|(k, _)| AppSecretOption { secret_name: k }).collect())
        }
    }
}

fn convert_services(yaml: HashMap<String, ServiceSpecYaml>) -> Result<Vec<ServiceSpec>> {
    let mut services = Vec::new();
    for (name, svc) in yaml {
        services.push(convert_service(name, svc)?);
    }
    Ok(services)
}

fn convert_service(name: String, yaml: ServiceSpecYaml) -> Result<ServiceSpec> {
    let service_type = match yaml.service_type {
        Some(ServiceTypeYaml::Public) => ServiceType::Public,
        Some(ServiceTypeYaml::Internal) => ServiceType::Internal,
        Some(ServiceTypeYaml::Job) => ServiceType::Job,
        None => ServiceType::Internal, // Default is internal
    };

    let mut image_variants = Vec::new();
    if let Some(img) = yaml.image {
        image_variants.push(ImageVariant {
            variant_name: "default".to_string(),
            image: img,
        });
    }
    if let Some(variants) = yaml.variants {
        for (v_name, v_img) in variants {
            image_variants.push(ImageVariant {
                variant_name: v_name,
                image: v_img.image,
            });
        }
    }

    let environment = yaml.environment.unwrap_or_default().into_iter().map(|s| {
        if s == "$all" {
            ServiceEnvOption::All
        } else if let Some((k, v)) = s.split_once('=') {
            ServiceEnvOption::WithValue(k.trim().to_string(), v.trim().to_string())
        } else {
            ServiceEnvOption::Simple(s)
        }
    }).collect();

    let configs = yaml.configs.unwrap_or_default().into_iter().flat_map(|map| {
        map.into_iter().map(|(k, v)| ServiceConfigOption {
            config_name: k,
            mount_path: v,
        })
    }).collect();

    let secrets = if let Some(secs) = yaml.secrets {
        convert_service_secrets(secs)?
    } else {
        vec![]
    };

    Ok(ServiceSpec {
        name,
        service_type,
        image_variants,
        environment,
        configs,
        secrets,
    })
}

fn convert_service_secrets(yaml: Vec<ServiceSecretYaml>) -> Result<Vec<ServiceSecret>> {
    let mut secrets = Vec::new();
    for s in yaml {
        match s {
            ServiceSecretYaml::Simple(name) => {
                secrets.push(ServiceSecret {
                    name: name.clone(),
                    mount: SecretMount::FilePath(format!("/secrets/{}", name))
                });
            }
            ServiceSecretYaml::Detailed(map) => {
                for (name, config) in map {
                    let mount = if let Some(c) = config {
                        if let Some(p) = c.path {
                            SecretMount::FilePath(p)
                        } else if let Some(e) = c.variable {
                            SecretMount::EnvVariable(e)
                        } else {
                            return Err(anyhow!("Secret {} must have either path: or variable: specified, or neigher", name));
                        }
                    } else {
                        return Err(anyhow!("Secret {} configuration is missing", name));
                    };

                    secrets.push(ServiceSecret {
                        name,
                        mount,
                    });
                }
            }
        }
    }
    Ok(secrets)
}

pub fn convert_env_spec(yaml: DeploymentEnvironmentSpecYaml, root: &Path) -> Result<DeploymentEnvironmentSpec> {
    let ingress_type_str = yaml.ingress.ingress_type.clone();
    let swarm_mode_opt = yaml.swarm_mode;
    let ingress = convert_ingress(yaml.ingress, &yaml.env_type)?;
    let registry = yaml.registry.unwrap_or_default();

    let mut deployments = Vec::new();
    for (name, dep) in &yaml.deployments {
        deployments.push(convert_deployment(name.clone(), dep, root)?);
    }

    let env_type = match yaml.env_type {
        DeploymentEnvTypeYaml::K8S => {
             if swarm_mode_opt.is_some() {
                 return Err(anyhow!("swarm_mode cannot be set for K8S environment"));
             }
             if ingress_type_str.is_some() {
                 return Err(anyhow!("ingress_type cannot be set for K8S environment"));
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
             if deployments.len() != 1 {
                 return Err(anyhow!("For Local environment exactly one deployment must be specified"));
             }

             // Check services ports
             let mut ports_seen = HashSet::new();
             for dep in &deployments {
                 if let Some(services) = &dep.services {
                     for (svc_name, svc_spec) in services {
                         if svc_spec.ports.is_empty() {
                             return Err(anyhow!("In Local environment, service {} must have at least one port", svc_name));
                         }
                         for port in &svc_spec.ports {
                             if !ports_seen.insert(port.external) {
                                 return Err(anyhow!("Duplicate external port {} in deployment", port.external));
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
            if t.disable == Some(true) {
                None
            } else {
                 let letsencrypt = if let Some(le) = t.letsencrypt {
                    Some(LetsEncryptSpec {
                        server: le.server,
                        email: le.email,
                    })
                } else {
                    None
                };
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

fn convert_env_variables(yaml: &Option<DeploymentEnvVariablesYaml>) -> Result<Vec<spec::EnvVariable>> {
    match yaml {
        Some(DeploymentEnvVariablesYaml::FromEnvFile(env_file)) =>  env_loader::load_env_file(env_file),
        Some(DeploymentEnvVariablesYaml::FromList(v)) => {
            v.into_iter()
                .map(|s| env_loader::parse_env_variable(&s))
                .collect::<Result<Vec<_>>>()
        },
        None => Ok(vec![]),
    }
}

fn convert_deployment(name: String, yaml: &DeploymentSpecYaml, root: &Path) -> Result<DeploymentSpec> {
    let application = convert_deployment_app(&yaml.application)?;

    let environment = convert_env_variables(&yaml.environment)?;

    let undockerized_environment = convert_env_variables(&yaml.undockerized_environment)?;

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
                 Err(anyhow!("Config path {} is not a directory", v))?
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
                DeploymentSecretSpecExYaml::Local(value) => {
                    list.push(DeploymentSecretSpec {
                        secret_name: k.clone(),
                        source: DeploymentSecretSource::Embedded(value.clone()),
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
             requests: ResourceLimits { memory: "128Mi".into(), cpu: "100m".into() },
             limits: ResourceLimits { memory: "128Mi".into(), cpu: "100m".into() }
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
        Some(semver::VersionReq::parse(&v)?)
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
             ResourceLimits { memory: "128Mi".into(), cpu: "100m".into() },
             ResourceLimits { memory: "128Mi".into(), cpu: "100m".into() },
         )
    };

    Ok(ResourcesSpec {
        replicas,
        requests,
        limits,
    })
}

fn convert_limits(yaml: Option<&ResourceLimitsYaml>) -> ResourceLimits {
    if let Some(l) = yaml {
        ResourceLimits {
            memory: l.memory.clone().unwrap_or("128Mi".into()),
            cpu: l.cpu.clone().unwrap_or("100m".into()),
        }
    } else {
        ResourceLimits { memory: "128Mi".into(), cpu: "100m".into() }
    }
}

fn convert_deployment_service(yaml: &DeploymentServiceSpecYaml, defaults: &ResourcesSpec) -> Result<DeploymentServiceSpec> {
    let mut prefixes = if let Some(p) = &yaml.prefixes {
        let mut res = Vec::new();
        for (k, v) in p {
                res.push(Prefix { prefix: k.clone(), strip: v.strip.unwrap_or(false) });
        }
        res
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

    let ports = if let Some(ports_yaml) = &yaml.ports {
        ports_yaml.into_iter().map(|s| {
            let (ext_str, int_str) = s.split_once(':')
                .ok_or_else(|| anyhow!("Invalid port format '{}'. Expected format 'external:internal'", s))?;

            let external = ext_str.parse::<u16>()
                .context(format!("Invalid external port '{}' in '{}'", ext_str, s))?;
            let internal = int_str.parse::<u16>()
                .context(format!("Invalid internal port '{}' in '{}'", int_str, s))?;

            Ok(ServicePort {
                external,
                internal,
            })
        }).collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };

    Ok(DeploymentServiceSpec {
        variant: yaml.variant.clone(),
        host: yaml.host.clone(),
        prefixes,
        resources,
        ports,
    })
}
