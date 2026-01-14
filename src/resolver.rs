use crate::spec::*;
use crate::spec::EnvVariable;
use crate::resolved_spec::*;
use anyhow::{Result, anyhow, Context};
use std::collections::{HashSet};
use std::fs;
use std::path::Path;
use std::env;

pub fn resolve(
    env_spec: &DeploymentEnvironmentSpec,
    app_spec: &AppSpec,
    deployment_name: &str
) -> Result<EnvironmentResolvedSpec> {
    let deployment = env_spec.deployments.iter()
        .find(|d| d.name == deployment_name)
        .ok_or_else(|| anyhow!("Deployment {} not found", deployment_name))?;

    // 1. Resolve Configs
    let mut resolved_configs = Vec::new();
    for config_spec in &deployment.configs {
        let mut resolved_files = Vec::new();
        for file_path in &config_spec.files {
            let path = Path::new(file_path);
            if !path.exists() {
                 return Err(anyhow!("Config file not found: {:?}", file_path));
            }
            if path.is_dir() {
                for entry in fs::read_dir(path)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                         let content = fs::read(&path).context(format!("Failed to read config file {:?}", path))?;
                         let name = path.file_name().unwrap().to_string_lossy().to_string();
                         resolved_files.push(ConfigResolvedFile { name, content });
                    }
                }
            } else {
                let content = fs::read(path).context(format!("Failed to read config file {:?}", path))?;
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                resolved_files.push(ConfigResolvedFile { name, content });
            }
        }
        resolved_configs.push(ConfigResolvedSpec {
            name: format!("{}-{}", app_spec.name, config_spec.name),
            files: resolved_files,
        });
    }

    // 2. Resolve Secrets
    let mut resolved_secrets = Vec::new();
    for secret_spec in &deployment.secrets {
        let value = match &secret_spec.source {
            DeploymentSecretSource::EnvVariable(var_name) => {
                env::var(var_name).context(format!("Secret environment variable {} not set", var_name))?
            }
            DeploymentSecretSource::FilePath(path_str) => {
                let path = Path::new(path_str);
                if !path.exists() {
                    return Err(anyhow!("Secret file not found: {:?}", path_str));
                }
                fs::read_to_string(path).context(format!("Failed to read secret file {:?}", path_str))?
            }
            DeploymentSecretSource::Embedded(value) => value.clone(),
        };
        resolved_secrets.push(SecretResolvedSpec {
            name: format!("{}-{}", app_spec.name, secret_spec.secret_name),
            value,
        });
    }

    // 3. Resolve Services
    let mut resolved_services = Vec::new();
    let mut public_host_prefix_combinations = HashSet::new();

    let primary_host = &deployment.primary_host;

    for app_service in app_spec.all_services() {
        let deployment_service_opt = deployment.services.as_ref().and_then(|s| s.get(&app_service.name));

        let defaults = &deployment.defaults;

        let empty_prefixes = Vec::new();
        let (variant_name,  prefixes, _resources) =
            if let Some(ds) = deployment_service_opt {
             (
                 ds.variant.as_deref().unwrap_or("default"),
                 &ds.prefixes,
                 &ds.resources
             )
        } else {
             ("default",  &empty_prefixes, defaults)
        };




        let mut host_name = primary_host.clone();
        if let Some(deployment_service) = deployment_service_opt {
            host_name = deployment_service.host.clone().unwrap_or(primary_host.clone());
        }

        let mut host_domain_name: &String;

        let host = env_spec.ingress.hosts.iter()
            .find(|host_spec| &(host_spec.name) == &host_name);

        match host.and_then(|h| h.domain_names.first()) {
            Some(host) => {
                host_domain_name = host;
            },
            None =>return Err(anyhow!("Host {} not found in ingress spec", host_name)),
        }

        let is_app_service = app_service.is_app_service;

        // Resolve Image
        let mut raw_image = app_service.image_variants.iter()
            .find(|v| v.variant_name == variant_name)
            .map(|v| v.image.clone())
            .ok_or_else(|| anyhow!("Image variant {} not found for service {}", variant_name, app_service.name))?;

        if is_app_service {
             if let DeploymentEnvType::Local = env_spec.env_type {
                  raw_image = format!("{}:latest", raw_image);
             } else {
                  raw_image = format!("{}:{}", raw_image, app_spec.version.to_string());
             }
        }

        if env_spec.env_type != DeploymentEnvType::Local && env_spec.registry.is_empty() {
            return Err(anyhow!("Registry mapping is required for non-local deployments"));
        }

        let image = if (is_app_service) {
            resolve_app_service_image(env_spec, raw_image)?
        } else {
            raw_image
        };

        // Check Public Service uniqueness
        if let ServiceType::Public = app_service.service_type {
            for prefix in prefixes {
                 let key = (host_name.to_string(), prefix.prefix.clone());
                 if !public_host_prefix_combinations.insert(key) {
                     return Err(anyhow!("Duplicate host+prefix combination for public service {}: {}{}",
                         app_service.name, host_name, prefix.prefix));
                 }
            }
        }

        // Resolve Environment Variables
        let environment_variables = resolve_app_env_vars(app_spec, &deployment.environment, Some(host_domain_name))?;
        let final_service_env_vars = filter_service_env_vars(app_service, &environment_variables)?;

        // Resolve Undockerized Environment Variables
        let mut undockerized_values = deployment.environment.clone();
        for override_var in &deployment.undockerized_environment {
            add_unique_var(&mut undockerized_values, override_var.clone());
        }
        let undockerized_variables = resolve_app_env_vars(app_spec, &undockerized_values, Some(host_domain_name))?;
        let final_undockerized_service_env_vars = filter_service_env_vars(app_service, &undockerized_variables)?;

        // Resolve Configs
        let mut service_configs = Vec::new();
        for sc_opt in &app_service.configs {
             let config_name = format!("{}-{}", app_spec.name, sc_opt.config_name);
             if !resolved_configs.iter().any(|c| c.name == config_name) {
                  return Err(anyhow!("Service {} references undefined config {}", app_service.name, config_name));
             }
             service_configs.push(ServiceConfigOption {
                 config_name,
                 mount_path: sc_opt.mount_path.clone(),
             });
        }

        // Resolve Secrets
        let mut service_secrets = Vec::new();
        for sec in &app_service.secrets {
             let secret_name = format!("{}-{}", app_spec.name, sec.name);
             if !resolved_secrets.iter().any(|s| s.name == secret_name) {
                  return Err(anyhow!("Service {} references undefined secret {}", app_service.name, secret_name));
             }
             service_secrets.push(ServiceSecret {
                 name: secret_name,
                 mount: sec.mount.clone(),
             });
        }

        resolved_services.push(ServiceResolvedSpec {
            full_name: format!("{}-{}", app_spec.name, app_service.name),
            service_type: app_service.service_type.clone(),
            image,
            service_host: host_domain_name.clone(),
            environment_variables: final_service_env_vars,
            undockerized_environment_variables: final_undockerized_service_env_vars,
            configs: service_configs,
            secrets: service_secrets,
            ports: deployment_service_opt.map(|s|
                s.ports.clone()
            ).unwrap_or(app_service.ports.clone()),
        });
    }

    let current_deployment = DeploymentResolvedSpec {
        name: deployment.name.clone(),
        application_name: deployment.application.name.clone(),
        configs: resolved_configs,
        secrets: resolved_secrets,
        defaults: deployment.defaults.clone(),
        services: resolved_services,
    };

    let mut ingress_rules = Vec::new();
    for host_spec in &env_spec.ingress.hosts {
        for domain in &host_spec.domain_names {
            let mut service_rules = Vec::new();

            for app_service in app_spec.all_services() {
                let deployment_service_opt = deployment.services.as_ref().and_then(|s| s.get(&app_service.name));
                if let Some(ds) = deployment_service_opt {
                    if  let ServiceType::Public = app_service.service_type {
                        let h = ds.host.clone().unwrap_or(primary_host.clone());
                        if &h == &host_spec.name {
                            let full_name = format!("{}-{}", app_spec.name, app_service.name);
                            // Determine port
                            let port = if let Some(_) = ds.ports.iter().find(|p| p.external == 80) {
                                80
                            } else if let Some(p) = ds.ports.first() {
                                p.external
                            } else {
                                80 // Default
                            };

                            for prefix in &ds.prefixes {
                                service_rules.push(IngressToServiceRule {
                                    service_name: full_name.clone(),
                                    port,
                                    prefix: prefix.prefix.clone(),
                                    strip_prefix: prefix.strip,
                                });
                            }
                        }
                    }
                }
            }

            if !service_rules.is_empty() {
                ingress_rules.push(IngressRule {
                    domain_name: domain.clone(),
                    services: service_rules,
                });
            }
        }
    }

    let tls = if let Some(tls_spec) = &env_spec.ingress.tls {
        let le_resolved = if let Some(le) = &tls_spec.letsencrypt {
             Some(LetsEncryptResolvedSpec {
                 server: le.server.clone().unwrap_or("https://acme-v02.api.letsencrypt.org/directory".to_string()),
                 email: le.email.clone(),
             })
        } else {
             None
        };
        Some(IngressTlsResolvedSpec {
            secret: tls_spec.secret.clone(),
            letsencrypt: le_resolved,
        })
    } else {
        None
    };

    let ingress_resolved = IngressResolvedSpec {
        name: env_spec.ingress.name.clone(),
        domains: env_spec.ingress.hosts.iter().flat_map(|h| h.domain_names.clone()).collect(),
        rules: ingress_rules,
        tls,
    };

    Ok(EnvironmentResolvedSpec {
        ingress: ingress_resolved,
        current_deployment,
        env_type: env_spec.env_type.clone(),
    })
}

fn resolve_app_service_image(env_spec: &DeploymentEnvironmentSpec, raw_image: String) -> Result<String> {
    let image = if let Some((namespace, _rest)) = raw_image.split_once('/') {
        if let Some(registry_host) = env_spec.registry.get(namespace) {
            let registry_host = registry_host.strip_suffix('/').unwrap_or(registry_host);
            format!("{}/{}", registry_host, raw_image)
        } else {
            if env_spec.env_type == DeploymentEnvType::Local {
                raw_image
            } else {
                let available: Vec<_> = env_spec.registry.keys().collect();
                return Err(anyhow!("Docker registry host for namespace '{}' not found in environment spec. Available namespaces: {:?}", namespace, available));
            }
        }
    } else {
        raw_image
    };
    Ok(image)
}

fn add_unique_var(vars: &mut Vec<EnvVariable>, var: EnvVariable) {
    if let Some(existing) = vars.iter_mut().find(|v| v.name == var.name) {
        existing.value = var.value;
    } else {
        vars.push(var);
    }
}

pub fn resolve_variable_in_string(input: &String, vars: &[EnvVariable]) -> Result<String> {
    let mut result = String::new();
    let mut last_end = 0;

    while let Some(start) = input[last_end..].find("${") {
        let absolute_start = last_end + start;
        result.push_str(&input[last_end..absolute_start]);

        if let Some(end_offset) = input[absolute_start..].find('}') {
            let absolute_end = absolute_start + end_offset;
            let var_name = &input[absolute_start + 2..absolute_end];

            if let Some(var) = vars.iter().find(|v| v.name == var_name) {
                result.push_str(&var.value);
            } else {
                return Err(anyhow!("Undefined variable: {}", var_name));
            }

            last_end = absolute_end + 1;
        } else {
            return Err(anyhow!("Invalid variable reference: {}", input));
        }
    }

    result.push_str(&input[last_end..]);
    Ok(result)
}

fn resolve_app_env_vars(
    app_spec: &AppSpec,
    deployment_values: &[EnvVariable],
    host_domain_name: Option<&String>
) -> Result<Vec<EnvVariable>> {
    let mut environment_variables = Vec::new();

    // External
    for external in &app_spec.environment.external {
         let val = deployment_values.iter()
             .find(|e| e.name == external.name)
             .map(|e| e.value.clone())
             .or_else(|| external.default.clone());

         if let Some(v) = val {
              add_unique_var(&mut environment_variables, EnvVariable{ name: external.name.clone(), value:v });
         } else {
              return Err(anyhow!("Missing external env variable: {}", external.name));
         }
    }

    // Optional
    for optional in &app_spec.environment.optional {
        let val = deployment_values.iter()
            .find(|e| e.name == optional.name)
            .map(|e| e.value.clone());

        if let Some(v) = val {
            add_unique_var(&mut environment_variables, EnvVariable{ name: optional.name.clone(), value:v });
        }
    }

    // Relative
    for relative in &app_spec.environment.relative {
         if let Some(h) = host_domain_name {
              let url = format!("https://{}{}", h, relative.relative_value);
              let value = resolve_variable_in_string(&url, &environment_variables)
                  .context(format!("Failed to resolve relative env variable {}", relative.name))?;
              add_unique_var(&mut environment_variables, EnvVariable{
                  name:relative.name.clone(),
                  value,
              });
         }
    }

    // Internal
    for internal in &app_spec.environment.internal {
        let value = resolve_variable_in_string(&internal.value, &environment_variables)
            .context(format!("Failed to resolve internal env variable {}", internal.name))?;
        add_unique_var(&mut environment_variables, EnvVariable{
            name: internal.name.clone(),
            value,
        });
    }

    Ok(environment_variables)
}

fn filter_service_env_vars(
    app_service: &ServiceSpec,
    all_env_vars: &[EnvVariable]
) -> Result<Vec<EnvVariable>> {
    let mut final_service_env_vars = Vec::new();

    for svc_env_opt in &app_service.environment {
         match svc_env_opt {
             ServiceEnvOption::All => {
                 for env_var in all_env_vars {
                     add_unique_var(&mut final_service_env_vars, env_var.clone());
                 }
             }
             ServiceEnvOption::Simple(name) => {
                 if let Some(env_var) = all_env_vars.iter().find(|e| &e.name == name) {
                     add_unique_var(&mut final_service_env_vars, env_var.clone());
                 } else {
                     return Err(anyhow!("Service {} references undefined env var {}", app_service.name, name));
                 }
             }
             ServiceEnvOption::WithValue(k, v) => {
                 add_unique_var(&mut final_service_env_vars,EnvVariable{
                     name: k.clone(),
                     value: resolve_variable_in_string(v, all_env_vars)
                         .context(format!("{}: Failed to resolve env var {}={}", app_service.name, k, v))?
                 });
             }
         }
    }
    Ok(final_service_env_vars)
}
