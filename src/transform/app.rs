use crate::spec::*;
use crate::spec_yaml::*;
use crate::env_loader::parse_env_string;
use crate::spec;
use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::fs;

pub fn convert_app_spec(yaml: AppSpecYaml, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<AppSpec> {
    let version = semver::Version::parse(&yaml.version)
        .context("Failed to parse app version")?;

    let mut environment = if let Some(env) = yaml.environment {
        convert_environment(env)?
    } else {
        AppEnvironment {
            external: vec![],
            optional: vec![],
            relative: vec![],
            internal: vec![],
        }
    };

    let mut secrets = if let Some(sec) = yaml.secrets {
        convert_secrets(sec)?
    } else {
        vec![]
    };

    let mut configs: Vec<ConfigSpec> = if let Some(conf) = yaml.configs {
        conf.into_iter().map(|(k, v)| ConfigSpec {
            name: k,
            files: v,
        }).collect()
    } else {
        vec![]
    };

    let app_services = if let Some(services) = yaml.app_services {
        convert_services(services, true)?
    } else {
        vec![]
    };

    let mut combined_extra_services = HashMap::new();
    let mut volumes: Vec<String> = yaml.volumes.unwrap_or_default();

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
                if let Some(extra_env) = extra_yaml.environment {
                    let converted = convert_environment(extra_env)?;
                    environment.external.extend(converted.external);
                    environment.optional.extend(converted.optional);
                    environment.relative.extend(converted.relative);
                    environment.internal.extend(converted.internal);
                }
                if let Some(extra_configs) = extra_yaml.configs {
                    configs.extend(extra_configs.into_iter().map(|(k, v)| ConfigSpec { name: k, files: v }));
                }
                if let Some(extra_secrets) = extra_yaml.secrets {
                    secrets.extend(convert_secrets(extra_secrets)?);
                }
                if let Some(extra_volumes) = extra_yaml.volumes {
                    volumes.extend(extra_volumes);
                }
            }
        }
    }

    if let Some(services) = yaml.extra_services {
        combined_extra_services.extend(services);
    }

    let extra_services = convert_services(combined_extra_services, false)?;

    for svc in app_services.iter().chain(extra_services.iter()) {
        for vol in &svc.volumes {
            if let ServiceVolumeType::Named(vol_name) = &vol.name {
                if !volumes.contains(vol_name) {
                    return Err(anyhow!(
                        "Service '{}' references named volume '{}' which is not declared in app volumes",
                        svc.name, vol_name
                    ));
                }
            }
        }
    }

    Ok(AppSpec {
        name: yaml.name,
        version,
        environment,
        app_services,
        extra_services,
        configs,
        secrets,
        volumes,
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

fn convert_services(yaml: HashMap<String, ServiceSpecYaml>, is_app_service: bool) -> Result<Vec<ServiceSpec>> {
    yaml.into_iter()
        .map(|(name, svc)| convert_service(name, svc, is_app_service))
        .collect()
}

fn convert_service(name: String, yaml: ServiceSpecYaml, is_app_service: bool) -> Result<ServiceSpec> {
    let service_type = match yaml.service_type {
        Some(ServiceTypeYaml::Public) => ServiceType::Public,
        Some(ServiceTypeYaml::Internal) => ServiceType::Internal,
        Some(ServiceTypeYaml::Job) => ServiceType::Job,
        None => ServiceType::Internal,
    };

    let image = match (yaml.image, yaml.variants) {
        (Some(img), None) => ImageSpec::Exact(img),
        (None, Some(variants)) => ImageSpec::Variants(
            variants.into_iter().map(|(variant_name, v)| ImageVariant {
                variant_name,
                image: v.image,
            }).collect()
        ),
        (Some(_), Some(_)) => return Err(anyhow!("Service '{}' cannot have both 'image' and 'variants'", name)),
        (None, None) => return Err(anyhow!("Service '{}' must specify either 'image' or 'variants'", name)),
    };

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

    let ports = super::parse_ports(&yaml.ports)?;

    let volumes = yaml.volumes.unwrap_or_default().into_iter()
        .map(|s| parse_service_volume(&s))
        .collect::<Result<Vec<_>>>()?;

     let command = yaml.command.map(convert_service_command);
     let entrypoint = yaml.entrypoint.map(convert_service_command);
     let healthcheck = yaml.healthcheck.map(convert_healthcheck).transpose()?;

    Ok(ServiceSpec {
        name,
        service_type,
        image,
        environment,
        configs,
        secrets,
        ports,
        volumes,
        command,
        entrypoint,
        healthcheck,
        is_app_service,
    })
}

fn convert_service_command(c: ServiceCommandYaml) -> ServiceCommand {
    match c {
        ServiceCommandYaml::Shell(s) => ServiceCommand::Shell(s),
        ServiceCommandYaml::Exec(v) => ServiceCommand::Exec(v),
    }
}

fn convert_healthcheck(yaml: HealthcheckYaml) -> Result<Healthcheck> {
    let disable = yaml.disable.unwrap_or(false);
    let test = match yaml.test {
        Some(HealthcheckTestYaml::Shell(s)) => HealthcheckTest::Shell(s),
        Some(HealthcheckTestYaml::Exec(v)) => HealthcheckTest::Exec(v),
        // `test` may be omitted only when the check is being disabled.
        None if disable => HealthcheckTest::Exec(vec!["NONE".to_string()]),
        None => return Err(anyhow!("healthcheck requires a 'test' unless 'disable: true' is set")),
    };
    Ok(Healthcheck {
        test,
        interval: yaml.interval,
        timeout: yaml.timeout,
        retries: yaml.retries,
        start_period: yaml.start_period,
        disable,
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
                    secrets.push(ServiceSecret { name, mount });
                }
            }
        }
    }
    Ok(secrets)
}

fn parse_service_volume(s: &str) -> Result<ServiceVolume> {
    let (vol_str, mount_path) = s.split_once(':')
        .ok_or_else(|| anyhow!("Invalid volume format '{}'. Expected 'name:mount_path' or './path:mount_path'", s))?;

    let name = if vol_str.starts_with('.') || vol_str.starts_with('/') {
        ServiceVolumeType::Path(vol_str.to_string())
    } else {
        ServiceVolumeType::Named(vol_str.to_string())
    };

    Ok(ServiceVolume {
        name,
        mount_path: mount_path.to_string(),
    })
}
