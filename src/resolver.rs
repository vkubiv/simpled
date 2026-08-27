use crate::spec::*;
use crate::spec::EnvVariable;
use crate::resolved_spec::*;
use crate::secret_fetch;
use anyhow::{Result, anyhow, Context};
use std::collections::{HashSet, HashMap};
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
    // Keyed by the secret's original (unprefixed) name so deployment env values
    // can reference them via `$secret(name)`.
    let mut secret_values: HashMap<String, String> = HashMap::new();
    // Names of the secrets that are only read on the deploy target, kept so a
    // `$secret(name)` reference to one can be rejected with a useful message.
    let mut deferred_secrets: HashSet<String> = HashSet::new();
    for secret_spec in &deployment.secrets {
        let value = match &secret_spec.source {
            DeploymentSecretSource::EnvVariable(var_name) => {
                let value = env::var(var_name).context(format!("Secret environment variable {} not set", var_name))?;
                if value.is_empty() {
                    return Err(anyhow!("Secret environment variable {} is empty", var_name));
                }
                SecretResolvedValue::Literal(value)
            }
            DeploymentSecretSource::FilePath(path_str) => {
                let path = Path::new(path_str);
                if !path.exists() {
                    return Err(anyhow!("Secret file not found: {:?}", path_str));
                }
                SecretResolvedValue::Literal(
                    fs::read_to_string(path).context(format!("Failed to read secret file {:?}", path_str))?
                )
            }
            DeploymentSecretSource::Embedded(value) => SecretResolvedValue::Literal(value.clone()),
            // A local deployment runs on this machine, so there is no deploy
            // target to defer to and no artifact for the value to leak into —
            // the lookup happens right here. Every other target gets the lookup
            // written into its generated `fetch-secrets.sh` instead.
            DeploymentSecretSource::Aws(reference) => {
                if env_spec.env_type == DeploymentEnvType::Local {
                    SecretResolvedValue::Literal(secret_fetch::fetch_locally(reference)?)
                } else {
                    SecretResolvedValue::Deferred(reference.clone())
                }
            }
        };
        match &value {
            SecretResolvedValue::Literal(literal) => {
                secret_values.insert(secret_spec.secret_name.clone(), literal.clone());
            }
            SecretResolvedValue::Deferred(_) => {
                deferred_secrets.insert(secret_spec.secret_name.clone());
            }
        }
        resolved_secrets.push(SecretResolvedSpec {
            name: format!("{}-{}", app_spec.name, secret_spec.secret_name),
            value,
        });
    }

    // Deployment-level env values may reference secrets via `$secret(name)`.
    // Expand those references once before the values feed into service resolution.
    let deployment_environment = substitute_secret_refs(&deployment.environment, &secret_values, &deferred_secrets)?;
    let deployment_undockerized_environment =
        substitute_secret_refs(&deployment.undockerized_environment, &secret_values, &deferred_secrets)?;

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
        let mut raw_image = match &app_service.image {
            ImageSpec::Exact(img) => img.clone(),
            ImageSpec::Variants(variants) => variants.iter()
                .find(|v| v.variant_name == variant_name)
                .map(|v| v.image.clone())
                .ok_or_else(|| anyhow!("Image variant '{}' not found for service '{}'", variant_name, app_service.name))?,
        };

        if is_app_service {
             if let DeploymentEnvType::Local = env_spec.env_type {
                  raw_image = format!("{}:latest", raw_image);
             } else {
                  raw_image = format!("{}:{}", raw_image, version_to_tag(&app_spec.version.to_string()));
             }
        }

        if env_spec.env_type != DeploymentEnvType::Local && env_spec.registry.is_empty() {
            return Err(anyhow!("Registry mapping is required for non-local deployments"));
        }

        let image = if is_app_service {
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
        let use_tls = env_spec.ingress.tls.is_some();
        let environment_variables = resolve_app_env_vars(app_spec, &deployment_environment, Some(host_domain_name), use_tls)?;
        let final_service_env_vars = filter_service_env_vars(app_service, app_spec, &environment_variables)?;

        // Resolve Undockerized Environment Variables
        let mut undockerized_values = deployment_environment.clone();
        for override_var in &deployment_undockerized_environment {
            add_unique_var(&mut undockerized_values, override_var.clone());
        }
        let undockerized_variables = resolve_app_env_vars(app_spec, &undockerized_values, Some(host_domain_name), use_tls)?;
        let final_undockerized_service_env_vars = filter_service_env_vars(app_service, app_spec, &undockerized_variables)?;

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

        // The deployment's volumes are appended to the service's own, so a
        // deployment can mount a source tree into a service (letting a watch
        // server rebuild it in place) without the app spec knowing about it.
        // A named volume still has to be declared in the app spec's top-level
        // `volumes:`, the same rule app-spec-declared mounts obey.
        let mut service_volumes = app_service.volumes.clone();
        if let Some(deployment_service) = deployment_service_opt {
            for volume in &deployment_service.volumes {
                if let ServiceVolumeType::Named(vol_name) = &volume.name {
                    if !app_spec.volumes.contains(vol_name) {
                        return Err(anyhow!(
                            "Deployment '{}' mounts named volume '{}' on service '{}', but it is not declared in app volumes",
                            deployment.name, vol_name, app_service.name
                        ));
                    }
                }
                service_volumes.push(volume.clone());
            }
        }

        resolved_services.push(ServiceResolvedSpec {
            full_name: format!("{}", app_service.name),
            service_type: app_service.service_type.clone(),
            is_app_service,
            image,
            service_host: host_domain_name.clone(),
            environment_variables: final_service_env_vars,
            undockerized_environment_variables: final_undockerized_service_env_vars,
            configs: service_configs,
            secrets: service_secrets,
            volumes: service_volumes,
            expose: app_service.expose.clone(),
            command: deployment_service_opt
                .and_then(|s| s.command.clone())
                .or_else(|| app_service.command.clone()),
            entrypoint: deployment_service_opt
                .and_then(|s| s.entrypoint.clone())
                .or_else(|| app_service.entrypoint.clone()),
            healthcheck: app_service.healthcheck.clone(),
            depends_on: app_service.depends_on.clone(),
            ports: deployment_service_opt.map(|s|
                s.ports.clone()
            ).unwrap_or(app_service.ports.clone()),
            working_dir: deployment_service_opt.and_then(|s| s.working_dir.clone()),
        });
    }

    let current_deployment = DeploymentResolvedSpec {
        name: deployment.name.clone(),
        application_name: deployment.application.name.clone(),
        configs: resolved_configs,
        secrets: resolved_secrets,
        defaults: deployment.defaults.clone(),
        services: resolved_services,
        volumes: app_spec.volumes.clone(),
    };

    // Validate that every Public service configured in the current deployment has at least one ingress rule.
    // A Public service with no prefixes is likely a configuration mistake.
    if let Some(dep_services) = &deployment.services {
        for app_service in app_spec.all_services() {
            if let ServiceType::Public = app_service.service_type {
                if let Some(ds) = dep_services.get(&app_service.name) {
                    if ds.prefixes.is_empty() {
                        return Err(anyhow!(
                            "Public service '{}' in deployment '{}' has no prefixes configured and will not be reachable via ingress.",
                            app_service.name,
                            deployment.name
                        ));
                    }
                }
            }
        }
    }

    let mut ingress_rules = Vec::new();
    for host_spec in &env_spec.ingress.hosts {
        for domain in &host_spec.domain_names {
            let mut service_rules = Vec::new();

            for dep in &env_spec.deployments {
                let dep_primary_host = &dep.primary_host;
                // Services live in a HashMap, so sort by name to keep the
                // generated ingress configuration byte-identical across runs.
                let mut dep_services: Vec<_> = dep.services.clone().unwrap_or_default().into_iter().collect();
                dep_services.sort_by(|a, b| a.0.cmp(&b.0));
                for (service_name, ds) in dep_services {
                        let h = ds.host.clone().unwrap_or(dep_primary_host.clone());
                        if &h == &host_spec.name {
                            let full_name = format!("{}", service_name);
                            // Determine port
                            let port = if let Some(_) = ds.ports.iter().find(|p| p.external == 80) {
                                80
                            } else if let Some(p) = ds.ports.first() {
                                p.external
                            } else {
                                80 // Default
                            };

                            // A service's own limit wins over the gateway-wide
                            // default; resolving it here means every generator
                            // sees one effective number per route.
                            let body_limit = ds.body_limit.or(env_spec.ingress.body_limit);

                            for prefix in &ds.prefixes {
                                service_rules.push(IngressToServiceRule {
                                    service_name: full_name.clone(),
                                    deployment_name: dep.name.clone(),
                                    port,
                                    prefix: prefix.prefix.clone(),
                                    strip_prefix: prefix.strip,
                                    body_limit,
                                });
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

    // Guard against ambiguous ingress routing: within a single domain, two
    // services mapping to the same path prefix cannot be disambiguated by a
    // host-based ingress (nginx/traefik/k8s) or the local gateway, so one route
    // would silently shadow the other. The same domain can be spread across
    // several rules (declared under multiple host groups), so aggregate the
    // prefixes by domain across all rules. Prefixes are normalized so that "",
    // "/", and a trailing-slash variant all compare equal.
    let normalize_prefix = |prefix: &str| -> String {
        if prefix.is_empty() || prefix == "/" {
            "/".to_string()
        } else {
            prefix.trim_end_matches('/').to_string()
        }
    };
    let mut seen_prefixes: HashMap<&str, HashMap<String, (&str, &str)>> = HashMap::new();
    for rule in &ingress_rules {
        let domain_prefixes = seen_prefixes.entry(rule.domain_name.as_str()).or_default();
        for svc in &rule.services {
            let normalized = normalize_prefix(&svc.prefix);
            if let Some((prev_dep, prev_svc)) = domain_prefixes.get(&normalized) {
                return Err(anyhow!(
                    "Ingress misconfiguration: domain '{}' maps path '{}' to multiple services ('{}/{}' and '{}/{}'); each domain and path must route to exactly one service",
                    rule.domain_name, normalized, prev_dep, prev_svc, svc.deployment_name, svc.service_name
                ));
            }
            domain_prefixes.insert(normalized, (svc.deployment_name.as_str(), svc.service_name.as_str()));
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

    let redirects = resolve_redirects(&env_spec.ingress)?;

    // Redirect sources are domains the gateway answers on without routing them to
    // a service, so they belong in `domains` (which drives the certificate) even
    // though they carry no rules.
    let mut domains: Vec<String> = env_spec.ingress.hosts.iter()
        .flat_map(|h| h.domain_names.clone())
        .collect();
    domains.extend(redirects.iter().map(|r| r.from_domain.clone()));

    let ingress_resolved = IngressResolvedSpec {
        name: env_spec.ingress.name.clone(),
        domains,
        rules: ingress_rules,
        redirects,
        tls,
    };

    Ok(EnvironmentResolvedSpec {
        ingress: ingress_resolved,
        current_deployment,
        env_type: env_spec.env_type.clone(),
    })
}

/// Flattens `gateway.redirects` into one rule per source domain.
///
/// A source that is also declared under `gateway.hosts` would shadow every route
/// on that domain, and a source declared twice has no defined winner, so both are
/// rejected here rather than silently resolved by whichever generator runs.
fn resolve_redirects(ingress: &IngressSpec) -> Result<Vec<RedirectRule>> {
    let served_domains: Vec<&str> = ingress.hosts.iter()
        .flat_map(|h| h.domain_names.iter().map(|d| d.as_str()))
        .collect();

    let mut redirects: Vec<RedirectRule> = Vec::new();
    for redirect in &ingress.redirects {
        for from in &redirect.from {
            if served_domains.contains(&from.as_str()) {
                return Err(anyhow!(
                    "Gateway redirect source '{}' is also declared under gateway.hosts; a domain cannot both serve traffic and redirect away from it",
                    from
                ));
            }
            if from == &redirect.to {
                return Err(anyhow!("Gateway redirect from '{}' points at itself", from));
            }
            if let Some(previous) = redirects.iter().find(|r| &r.from_domain == from) {
                return Err(anyhow!(
                    "Gateway redirect source '{}' is declared twice (to '{}' and to '{}')",
                    from, previous.to, redirect.to
                ));
            }
            redirects.push(RedirectRule {
                from_domain: from.clone(),
                to: redirect.to.clone(),
                permanent: redirect.permanent,
            });
        }
    }

    Ok(redirects)
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

/// Expands `$secret(name)` references in a string with the resolved secret value.
/// `secrets` is keyed by the secret's original (unprefixed) name. Referencing an
/// unknown secret is an error, as is referencing one in `deferred` — a secret
/// whose value is only fetched on the deploy target, and so is not available to
/// substitute into the env files written here.
fn resolve_secret_refs(
    input: &str,
    secrets: &HashMap<String, String>,
    deferred: &HashSet<String>,
) -> Result<String> {
    const MARKER: &str = "$secret(";
    let mut result = String::new();
    let mut last_end = 0;

    while let Some(start) = input[last_end..].find(MARKER) {
        let absolute_start = last_end + start;
        result.push_str(&input[last_end..absolute_start]);

        let name_start = absolute_start + MARKER.len();
        if let Some(close_offset) = input[name_start..].find(')') {
            let name_end = name_start + close_offset;
            let secret_name = &input[name_start..name_end];

            match secrets.get(secret_name) {
                Some(value) => result.push_str(value),
                None if deferred.contains(secret_name) => return Err(anyhow!(
                    "$secret({}) cannot be used: the secret has an 'aws' source, so its value is \
                     only fetched on the deploy target and cannot be substituted into an env \
                     variable here. Mount it on the service with `variable:` instead.",
                    secret_name
                )),
                None => return Err(anyhow!("Undefined secret reference: $secret({})", secret_name)),
            }

            last_end = name_end + 1;
        } else {
            return Err(anyhow!("Invalid secret reference (missing ')'): {}", input));
        }
    }

    result.push_str(&input[last_end..]);
    Ok(result)
}

/// Applies `resolve_secret_refs` to every value in an env variable list.
fn substitute_secret_refs(
    vars: &[EnvVariable],
    secrets: &HashMap<String, String>,
    deferred: &HashSet<String>,
) -> Result<Vec<EnvVariable>> {
    vars.iter()
        .map(|v| {
            Ok(EnvVariable {
                name: v.name.clone(),
                value: resolve_secret_refs(&v.value, secrets, deferred)?,
            })
        })
        .collect()
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
    host_domain_name: Option<&String>,
    use_tls: bool,
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
              let scheme = if use_tls { "https" } else { "http" };
              let url = format!("{}://{}{}", scheme, h, relative.relative_value);
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

/// Collects the names in well-formed `${...}` references. Malformed input is
/// left to `resolve_variable_in_string`, which reports it properly.
fn referenced_var_names(input: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = input;

    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                names.push(&after[..end]);
                rest = &after[end + 1..];
            }
            None => break,
        }
    }

    names
}

fn filter_service_env_vars(
    app_service: &ServiceSpec,
    app_spec: &AppSpec,
    all_env_vars: &[EnvVariable]
) -> Result<Vec<EnvVariable>> {
    // Optional variables this environment did not provide. A service may reference
    // one either directly or through `${...}`; the entry is then left off the
    // service instead of failing the deployment, which is what makes it optional.
    let unset_optional: HashSet<&str> = app_spec.environment.optional.iter()
        .map(|o| o.name.as_str())
        .filter(|name| !all_env_vars.iter().any(|e| e.name == *name))
        .collect();

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
                 } else if !unset_optional.contains(name.as_str()) {
                     return Err(anyhow!("Service {} references undefined env var {}", app_service.name, name));
                 }
             }
             ServiceEnvOption::WithValue(k, v) => {
                 if referenced_var_names(v).iter().any(|n| unset_optional.contains(n)) {
                     continue;
                 }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ingress_with(hosts: &[(&str, &[&str])], redirects: Vec<RedirectSpec>) -> IngressSpec {
        IngressSpec {
            name: "gateway".to_string(),
            hosts: hosts.iter()
                .map(|(name, domains)| HostSpec {
                    name: name.to_string(),
                    domain_names: domains.iter().map(|d| d.to_string()).collect(),
                })
                .collect(),
            tls: None,
            redirects,
            body_limit: None,
        }
    }

    fn redirect(from: &[&str], to: &str, permanent: bool) -> RedirectSpec {
        RedirectSpec {
            from: from.iter().map(|f| f.to_string()).collect(),
            to: to.to_string(),
            permanent,
        }
    }

    #[test]
    fn a_redirect_becomes_one_rule_per_source_domain() {
        let ingress = ingress_with(
            &[("web", &["www.somesite.com"])],
            vec![redirect(&["somesite.com", "somesite.net"], "www.somesite.com", true)],
        );

        let rules = resolve_redirects(&ingress).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].from_domain, "somesite.com");
        assert_eq!(rules[1].from_domain, "somesite.net");
        assert!(rules.iter().all(|r| r.to == "www.somesite.com" && r.permanent));
    }

    #[test]
    fn a_redirect_source_that_is_also_served_is_rejected() {
        let ingress = ingress_with(
            &[("web", &["www.somesite.com", "somesite.com"])],
            vec![redirect(&["somesite.com"], "www.somesite.com", true)],
        );

        let err = resolve_redirects(&ingress).unwrap_err().to_string();
        assert!(err.contains("somesite.com"), "unexpected error: {}", err);
        assert!(err.contains("gateway.hosts"), "unexpected error: {}", err);
    }

    #[test]
    fn the_same_source_cannot_redirect_to_two_places() {
        let ingress = ingress_with(
            &[("web", &["www.somesite.com"])],
            vec![
                redirect(&["somesite.com"], "www.somesite.com", true),
                redirect(&["somesite.com"], "other.com", true),
            ],
        );

        let err = resolve_redirects(&ingress).unwrap_err().to_string();
        assert!(err.contains("declared twice"), "unexpected error: {}", err);
    }

    #[test]
    fn a_redirect_to_itself_is_rejected() {
        let ingress = ingress_with(
            &[("web", &["www.somesite.com"])],
            vec![redirect(&["somesite.com"], "somesite.com", true)],
        );

        let err = resolve_redirects(&ingress).unwrap_err().to_string();
        assert!(err.contains("points at itself"), "unexpected error: {}", err);
    }

    #[test]
    fn a_bare_target_domain_picks_up_the_gateway_scheme() {
        let rule = RedirectRule {
            from_domain: "somesite.com".to_string(),
            to: "www.somesite.com".to_string(),
            permanent: false,
        };
        assert_eq!(rule.target_url(true), "https://www.somesite.com");
        assert_eq!(rule.target_url(false), "http://www.somesite.com");
        assert_eq!(rule.status_code(), 302);

        let absolute = RedirectRule {
            from_domain: "somesite.com".to_string(),
            to: "https://elsewhere.example/".to_string(),
            permanent: true,
        };
        assert_eq!(absolute.target_url(false), "https://elsewhere.example");
        assert_eq!(absolute.status_code(), 301);
    }

    fn secrets() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("postgres_password".to_string(), "s3cr3t".to_string());
        m
    }

    fn expand(input: &str) -> Result<String> {
        resolve_secret_refs(input, &secrets(), &HashSet::new())
    }

    #[test]
    fn expands_secret_reference() {
        let out = expand("postgresql://postgres:$secret(postgres_password)@postgres:5432/hobbyshopify").unwrap();
        assert_eq!(out, "postgresql://postgres:s3cr3t@postgres:5432/hobbyshopify");
    }

    #[test]
    fn expands_multiple_references() {
        let out = expand("$secret(postgres_password)-$secret(postgres_password)").unwrap();
        assert_eq!(out, "s3cr3t-s3cr3t");
    }

    #[test]
    fn passes_through_without_reference() {
        let out = expand("plain-value").unwrap();
        assert_eq!(out, "plain-value");
    }

    fn optional_app_spec(optional: &[&str]) -> AppSpec {
        AppSpec {
            name: "shop".to_string(),
            version: semver::Version::new(1, 0, 0),
            environment: AppEnvironment {
                external: vec![],
                optional: optional.iter()
                    .map(|n| OptionalEnvVariable { name: n.to_string() })
                    .collect(),
                relative: vec![],
                internal: vec![],
            },
            app_services: vec![],
            extra_services: vec![],
            configs: vec![],
            secrets: vec![],
            volumes: vec![],
        }
    }

    fn service_with_env(environment: Vec<ServiceEnvOption>) -> ServiceSpec {
        ServiceSpec {
            name: "backend".to_string(),
            service_type: ServiceType::Internal,
            is_app_service: true,
            image: ImageSpec::Exact("org/backend".to_string()),
            environment,
            configs: vec![],
            secrets: vec![],
            ports: vec![],
            expose: vec![],
            volumes: vec![],
            command: None,
            entrypoint: None,
            healthcheck: None,
            depends_on: vec![],
        }
    }

    #[test]
    fn unset_optional_var_is_left_off_the_service() {
        let app_spec = optional_app_spec(&["LIVEKIT_URL", "LIVEKIT_API_KEY"]);
        let service = service_with_env(vec![
            ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string()),
            ServiceEnvOption::WithValue("URL".to_string(), "${LIVEKIT_URL}".to_string()),
        ]);

        let vars = filter_service_env_vars(&service, &app_spec, &[]).unwrap();
        assert!(vars.is_empty(), "unset optional vars must not reach the service: {:?}", vars);
    }

    #[test]
    fn provided_optional_var_reaches_the_service() {
        let app_spec = optional_app_spec(&["LIVEKIT_URL", "LIVEKIT_API_KEY"]);
        let service = service_with_env(vec![
            ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string()),
            ServiceEnvOption::WithValue("URL".to_string(), "${LIVEKIT_URL}".to_string()),
        ]);
        let all = vec![
            EnvVariable { name: "LIVEKIT_API_KEY".to_string(), value: "key".to_string() },
            EnvVariable { name: "LIVEKIT_URL".to_string(), value: "wss://lk".to_string() },
        ];

        let vars = filter_service_env_vars(&service, &app_spec, &all).unwrap();
        assert_eq!(vars.len(), 2);
        assert_eq!(vars.iter().find(|v| v.name == "LIVEKIT_API_KEY").unwrap().value, "key");
        assert_eq!(vars.iter().find(|v| v.name == "URL").unwrap().value, "wss://lk");
    }

    #[test]
    fn a_variable_that_is_not_optional_still_errors_when_missing() {
        let app_spec = optional_app_spec(&[]);
        let service = service_with_env(vec![ServiceEnvOption::Simple("REDIS_URL".to_string())]);

        let err = filter_service_env_vars(&service, &app_spec, &[]).unwrap_err();
        assert!(err.to_string().contains("references undefined env var REDIS_URL"));
    }

    #[test]
    fn errors_on_unknown_secret() {
        let err = expand("$secret(missing)").unwrap_err();
        assert!(err.to_string().contains("Undefined secret reference"));
    }

    #[test]
    fn errors_on_unterminated_reference() {
        let err = expand("$secret(postgres_password").unwrap_err();
        assert!(err.to_string().contains("Invalid secret reference"));
    }

    /// A secret that is only fetched on the deploy target has no value here, so
    /// the reference has to fail with an explanation rather than as "undefined".
    #[test]
    fn errors_on_reference_to_a_deferred_secret() {
        let deferred = HashSet::from(["api_key".to_string()]);
        let err = resolve_secret_refs("$secret(api_key)", &secrets(), &deferred).unwrap_err();
        assert!(err.to_string().contains("'aws' source"));
    }
}
