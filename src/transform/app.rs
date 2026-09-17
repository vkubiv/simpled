use crate::env_loader::parse_env_string;
use crate::spec;
use crate::spec::*;
use crate::spec_yaml::*;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;

pub fn convert_app_spec(yaml: AppSpecYaml, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<AppSpec> {
    let version = semver::Version::parse(&yaml.version).context("Failed to parse app version")?;

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
        conf.into_iter()
            .map(|(k, v)| ConfigSpec { name: k, files: v })
            .collect()
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
                        svc.name,
                        vol_name
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
    let external = yaml
        .external
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let desc = parse_env_string(&s)?;
            Ok(ExternalEnvVariable {
                name: desc.name,
                default: desc.default,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let optional = yaml
        .optional
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let desc = parse_env_string(&s)?;
            if desc.default.is_some() {
                return Err(anyhow!(
                    "Optional env variable {} cannot have a default value",
                    desc.name
                ));
            }
            Ok(OptionalEnvVariable { name: desc.name })
        })
        .collect::<Result<Vec<_>>>()?;

    // A name in both lists would be required and optional at once; the
    // external entry would win and the optional one would be a silent no-op.
    for opt in &optional {
        if external.iter().any(|e| e.name == opt.name) {
            return Err(anyhow!(
                "Env variable {} is declared both as external and optional; declare it in one list only",
                opt.name
            ));
        }
    }

    let relative = yaml
        .relative
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let desc = parse_env_string(&s).map_err(|_| anyhow!("Invalid relative env variable format: {}", s))?;
            let val = desc
                .default
                .ok_or_else(|| anyhow!("Invalid relative env variable format (no value): {}", s))?;
            if !val.starts_with('/') {
                return Err(anyhow!("Relative URL for {} must start with /", desc.name));
            }
            Ok(RelativeEnvVariable {
                name: desc.name,
                relative_value: val,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let internal = yaml
        .internal
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let desc = parse_env_string(&s).map_err(|_| anyhow!("Invalid internal env variable format: {}", s))?;
            if let Some(val) = desc.default {
                Ok(InternalEnvVariable {
                    name: desc.name,
                    value: val,
                })
            } else {
                Err(anyhow!("Internal env variable {} must have a value", desc.name))
            }
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(AppEnvironment {
        external,
        optional,
        relative,
        internal,
    })
}

fn convert_secrets(yaml: AppSecretsYaml) -> Result<Vec<AppSecretOption>> {
    match yaml {
        AppSecretsYaml::Simple(list) => Ok(list.into_iter().map(|s| AppSecretOption { secret_name: s }).collect()),
        AppSecretsYaml::Detailed(map) => Ok(map.into_keys().map(|k| AppSecretOption { secret_name: k }).collect()),
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
            variants
                .into_iter()
                .map(|(variant_name, v)| ImageVariant {
                    variant_name,
                    image: v.image,
                })
                .collect(),
        ),
        (Some(_), Some(_)) => return Err(anyhow!("Service '{}' cannot have both 'image' and 'variants'", name)),
        (None, None) => return Err(anyhow!("Service '{}' must specify either 'image' or 'variants'", name)),
    };

    let environment = yaml
        .environment
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            if s == "$all" {
                ServiceEnvOption::All
            } else if let Some((k, v)) = s.split_once('=') {
                ServiceEnvOption::WithValue(k.trim().to_string(), v.trim().to_string())
            } else {
                ServiceEnvOption::Simple(s)
            }
        })
        .collect();

    let configs = yaml
        .configs
        .unwrap_or_default()
        .into_iter()
        .flat_map(|map| {
            map.into_iter().map(|(k, v)| ServiceConfigOption {
                config_name: k,
                mount_path: v,
            })
        })
        .collect();

    let secrets = if let Some(secs) = yaml.secrets {
        convert_service_secrets(secs)?
    } else {
        vec![]
    };

    let ports = super::parse_ports(&yaml.ports)?;

    let expose = yaml.expose.unwrap_or_default();

    let volumes = yaml
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|s| super::parse_service_volume(&s))
        .collect::<Result<Vec<_>>>()?;

    let command = yaml.command.map(super::convert_service_command);
    let entrypoint = yaml.entrypoint.map(super::convert_service_command);
    let healthcheck = yaml.healthcheck.map(convert_healthcheck).transpose()?;

    let depends_on = yaml.depends_on.unwrap_or_default();
    if depends_on.iter().any(|d| d == &name) {
        return Err(anyhow!("Service '{}' cannot depend on itself", name));
    }

    Ok(ServiceSpec {
        name,
        service_type,
        image,
        environment,
        configs,
        secrets,
        ports,
        expose,
        volumes,
        command,
        entrypoint,
        healthcheck,
        depends_on,
        is_app_service,
    })
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
                    mount: SecretMount::FilePath(format!("/secrets/{}", name)),
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
                            return Err(anyhow!(
                                "Secret {} must have either path: or variable: specified, or neigher",
                                name
                            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(raw: &str) -> Result<AppSpec> {
        let yaml: AppSpecYaml = serde_yaml::from_str(raw).unwrap();
        convert_app_spec(yaml, None)
    }

    #[test]
    fn a_variable_cannot_be_both_external_and_optional() {
        let err = convert(
            r#"
name: app
version: 1.0.0
environment:
  external:
    - LIVEKIT_URL
  optional:
    - LIVEKIT_URL
"#,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("both as external and optional"), "unexpected error: {err}");
    }

    #[test]
    fn distinct_external_and_optional_variables_are_accepted() {
        let spec = convert(
            r#"
name: app
version: 1.0.0
environment:
  external:
    - DB_URL
  optional:
    - LIVEKIT_URL
"#,
        )
        .unwrap();
        assert_eq!(spec.environment.external[0].name, "DB_URL");
        assert_eq!(spec.environment.optional[0].name, "LIVEKIT_URL");
    }

    fn convert_err(raw: &str) -> String {
        convert(raw).unwrap_err().to_string()
    }

    #[test]
    fn a_service_needs_exactly_one_of_image_and_variants() {
        let both = "name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n    variants:\n      arm:\n        image: b\n";
        assert!(convert_err(both).contains("cannot have both 'image' and 'variants'"));
        let neither = "name: app\nversion: 1.0.0\napp_services:\n  api:\n    type: public\n";
        assert!(convert_err(neither).contains("must specify either 'image' or 'variants'"));
    }

    #[test]
    fn a_named_volume_must_be_declared_at_the_top_level() {
        let raw = "name: app\nversion: 1.0.0\napp_services:\n  db:\n    image: postgres:16\n    volumes:\n      - pgdata:/var/lib/postgresql\n";
        assert!(convert_err(raw).contains("named volume 'pgdata' which is not declared"));

        let spec = convert(&format!("{raw}volumes:\n  - pgdata\n")).unwrap();
        assert!(matches!(&spec.app_services[0].volumes[0].name, ServiceVolumeType::Named(n) if n == "pgdata"));
        // Host paths need no declaration.
        let host = "name: app\nversion: 1.0.0\napp_services:\n  db:\n    image: postgres:16\n    volumes:\n      - ./data:/data\n";
        assert!(
            matches!(&convert(host).unwrap().app_services[0].volumes[0].name, ServiceVolumeType::Path(p) if p == "./data")
        );
    }

    #[test]
    fn a_healthcheck_needs_a_test_unless_it_is_disabled() {
        let raw =
            "name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n    healthcheck:\n      interval: 5s\n";
        assert!(convert_err(raw).contains("healthcheck requires a 'test'"));

        let disabled =
            "name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n    healthcheck:\n      disable: true\n";
        let spec = convert(disabled).unwrap();
        assert!(spec.app_services[0].healthcheck.as_ref().unwrap().is_disabled());
    }

    #[test]
    fn a_service_cannot_depend_on_itself() {
        let raw = "name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n    depends_on:\n      - api\n";
        assert!(convert_err(raw).contains("cannot depend on itself"));
    }

    #[test]
    fn service_environment_entries_take_three_forms() {
        let raw = "name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n    environment:\n      - $all\n      - PLAIN\n      - NAME = value with spaces \n";
        let spec = convert(raw).unwrap();
        let env = &spec.app_services[0].environment;
        assert!(matches!(env[0], ServiceEnvOption::All));
        assert!(matches!(&env[1], ServiceEnvOption::Simple(n) if n == "PLAIN"));
        assert!(matches!(&env[2], ServiceEnvOption::WithValue(k, v) if k == "NAME" && v == "value with spaces"));
    }

    #[test]
    fn service_secrets_default_to_a_file_under_secrets() {
        let raw = "name: app\nversion: 1.0.0\nsecrets:\n  - a\n  - b\napp_services:\n  api:\n    image: a\n    secrets:\n      - a\n      - b:\n          variable: B\n";
        let spec = convert(raw).unwrap();
        let secrets = &spec.app_services[0].secrets;
        assert!(matches!(&secrets[0].mount, SecretMount::FilePath(p) if p == "/secrets/a"));
        assert!(matches!(&secrets[1].mount, SecretMount::EnvVariable(v) if v == "B"));
    }

    #[test]
    fn relative_and_internal_variables_are_checked_for_shape() {
        let bad_relative = "name: app\nversion: 1.0.0\nenvironment:\n  relative:\n    - API_URL=api\n";
        assert!(convert_err(bad_relative).contains("must start with /"));
        let no_value = "name: app\nversion: 1.0.0\nenvironment:\n  internal:\n    - QUEUE\n";
        assert!(convert_err(no_value).contains("must have a value"));
        let optional_default = "name: app\nversion: 1.0.0\nenvironment:\n  optional:\n    - FLAG=1\n";
        assert!(convert_err(optional_default).contains("cannot have a default value"));
    }

    /// `application.extra` files add to every part of the app spec for one
    /// deployment, without the app repository knowing about them.
    #[test]
    fn extra_files_extend_the_app_spec_for_the_deployment() {
        use crate::test_support::env_spec;
        use std::fs;

        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("extra.yaml"),
            "extra_services:\n  cache:\n    image: redis:7\n    volumes:\n      - redisdata:/data\nenvironment:\n  external:\n    - CACHE_URL\nconfigs:\n  cache:\n    - redis.conf\nsecrets:\n  - cache_password\nvolumes:\n  - redisdata\n",
        )
        .unwrap();
        let env = env_spec(
            "type: k8s\ngateway:\n  hosts:\n    web: shop.example.com\n  tls:\n    disable: true\ndeployments:\n  prod:\n    primary_host: web\n    application:\n      name: app\n      extra:\n        - extra.yaml\n",
            root.path(),
        );

        let yaml: AppSpecYaml =
            serde_yaml::from_str("name: app\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n").unwrap();
        let spec = convert_app_spec(yaml, Some(&env)).unwrap();

        assert_eq!(spec.extra_services[0].name, "cache");
        assert!(!spec.extra_services[0].is_app_service);
        assert_eq!(spec.environment.external[0].name, "CACHE_URL");
        assert_eq!(spec.configs[0].name, "cache");
        assert_eq!(spec.secrets[0].secret_name, "cache_password");
        assert_eq!(spec.volumes, vec!["redisdata"]);

        // A deployment for another application does not pull the file in.
        let yaml: AppSpecYaml =
            serde_yaml::from_str("name: other\nversion: 1.0.0\napp_services:\n  api:\n    image: a\n").unwrap();
        let spec = convert_app_spec(yaml, Some(&env)).unwrap();
        assert!(spec.extra_services.is_empty());
    }
}
