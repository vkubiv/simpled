use crate::resolved_spec::*;
use crate::secret_fetch;
use crate::spec::EnvVariable;
use crate::spec::*;
use anyhow::{anyhow, Context, Result};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::Path;

pub fn resolve(
    env_spec: &DeploymentEnvironmentSpec,
    app_spec: &AppSpec,
    deployment_name: &str,
) -> Result<EnvironmentResolvedSpec> {
    let deployment = env_spec.deployment(deployment_name)?;

    if env_spec.env_type != DeploymentEnvType::Local && env_spec.registry.is_empty() {
        return Err(anyhow!("Registry mapping is required for non-local deployments"));
    }

    let configs = resolve_configs(deployment, app_spec)?;
    let secrets = resolve_secrets(env_spec, deployment, app_spec)?;

    // Deployment-level env values may reference secrets via `$secret(name)`.
    // Expand those references once before the values feed into service resolution.
    let deployment_environment = substitute_secret_refs(&deployment.environment, &secrets.values, &secrets.deferred)?;
    let deployment_undockerized_environment =
        substitute_secret_refs(&deployment.undockerized_environment, &secrets.values, &secrets.deferred)?;
    // The undockerized environment is the deployment environment with the
    // undockerized overrides applied on top.
    let mut undockerized_values = deployment_environment.clone();
    for override_var in &deployment_undockerized_environment {
        add_unique_var(&mut undockerized_values, override_var.clone());
    }

    let mut services = ServiceResolver {
        env_spec,
        app_spec,
        deployment,
        configs: &configs,
        secrets: &secrets.specs,
        deployment_environment: &deployment_environment,
        undockerized_values: &undockerized_values,
        env_by_host: HashMap::new(),
        public_routes: HashSet::new(),
    };
    let resolved_services = app_spec
        .all_services()
        .map(|app_service| services.resolve_service(app_service))
        .collect::<Result<Vec<_>>>()?;

    check_public_services_are_routed(deployment, app_spec)?;

    let current_deployment = DeploymentResolvedSpec {
        name: deployment.name.clone(),
        application_name: deployment.application.name.clone(),
        configs,
        secrets: secrets.specs,
        services: resolved_services,
        volumes: app_spec.volumes.clone(),
        host_environment: undockerized_values,
    };

    Ok(EnvironmentResolvedSpec {
        ingress: resolve_ingress(env_spec)?,
        current_deployment,
        env_type: env_spec.env_type.clone(),
    })
}

/// Reads every config file the deployment provides. Names are prefixed with the
/// application name, which is how services refer to them from here on.
fn resolve_configs(deployment: &DeploymentSpec, app_spec: &AppSpec) -> Result<Vec<ConfigResolvedSpec>> {
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
                    let path = entry?.path();
                    if path.is_file() {
                        resolved_files.push(read_config_file(&path)?);
                    }
                }
            } else {
                resolved_files.push(read_config_file(path)?);
            }
        }
        resolved_configs.push(ConfigResolvedSpec {
            name: format!("{}-{}", app_spec.name, config_spec.name),
            files: resolved_files,
        });
    }
    Ok(resolved_configs)
}

fn read_config_file(path: &Path) -> Result<ConfigResolvedFile> {
    let content = fs::read(path).context(format!("Failed to read config file {:?}", path))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .ok_or_else(|| anyhow!("Config path {:?} has no file name", path))?;
    Ok(ConfigResolvedFile { name, content })
}

/// A local deployment runs on this machine, so there is no deploy target to defer
/// to and no artifact for the value to leak into — the lookup happens right here.
/// Every other target gets the lookup written into its generated
/// `fetch-secrets.sh` instead.
fn resolve_aws_secret(env_spec: &DeploymentEnvironmentSpec, reference: &AwsSecretRef) -> Result<SecretResolvedValue> {
    if env_spec.env_type == DeploymentEnvType::Local {
        Ok(SecretResolvedValue::Literal(secret_fetch::fetch_locally(reference)?))
    } else {
        Ok(SecretResolvedValue::Deferred(reference.clone()))
    }
}

/// The variable named by `name` whatever case it is written in, with the name it
/// actually has. A `secrets_env_prefix` variable is spelled from a secret name,
/// and the two conventions disagree: `openai_api_key` against `E2E_SECRET_OPENAI_API_KEY`.
fn env_var_ignoring_case(name: &str) -> Result<Option<(String, String)>> {
    let mut found: Option<(String, String)> = None;
    for (key, value) in env::vars_os() {
        let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else {
            continue;
        };
        if !key.eq_ignore_ascii_case(name) {
            continue;
        }
        if let Some((first, _)) = &found {
            // Possible only where the environment is case-sensitive. Taking either
            // one would make the deployed value depend on iteration order.
            return Err(anyhow!(
                "Environment variables {} and {} both match the secret variable {}. \
                 Unset one of them.",
                first,
                key,
                name
            ));
        }
        found = Some((key.to_string(), value.to_string()));
    }
    Ok(found)
}

/// The deployment's secrets, plus the two views service resolution needs of them.
struct ResolvedSecrets {
    specs: Vec<SecretResolvedSpec>,
    /// Keyed by the secret's original (unprefixed) name so deployment env values
    /// can reference them via `$secret(name)`.
    values: HashMap<String, String>,
    /// Names of the secrets that are only read on the deploy target, kept so a
    /// `$secret(name)` reference to one can be rejected with a useful message.
    deferred: HashSet<String>,
}

fn resolve_secrets(
    env_spec: &DeploymentEnvironmentSpec,
    deployment: &DeploymentSpec,
    app_spec: &AppSpec,
) -> Result<ResolvedSecrets> {
    let mut secrets = ResolvedSecrets {
        specs: Vec::new(),
        values: HashMap::new(),
        deferred: HashSet::new(),
    };
    for secret_spec in &deployment.secrets {
        let value = match &secret_spec.source {
            DeploymentSecretSource::EnvVariable(var_name) => {
                let value = env::var(var_name).context(format!("Secret environment variable {} not set", var_name))?;
                let value = trim_secret_value(&value);
                if value.is_empty() {
                    return Err(anyhow!("Secret environment variable {} is empty", var_name));
                }
                SecretResolvedValue::Literal(value.to_string())
            }
            // `secrets_env_prefix`, so the variable's name was derived rather than
            // written down. Everything after the lookup is the `env:` source's.
            DeploymentSecretSource::PrefixedEnvVariable(lookup) => {
                match env_var_ignoring_case(&lookup.variable)? {
                    Some((var_name, value)) => {
                        let value = trim_secret_value(&value);
                        if value.is_empty() {
                            return Err(anyhow!("Secret environment variable {} is empty", var_name));
                        }
                        SecretResolvedValue::Literal(value.to_string())
                    }
                    // `secrets_aws` ends the chain: its lookup belongs on the deploy
                    // target, so nothing after it could be tried here anyway.
                    None => match &lookup.fallback {
                        Some(reference) => resolve_aws_secret(env_spec, reference)?,
                        None => {
                            let mut tried = lookup.tried.clone();
                            tried.push(format!("${} (not set)", lookup.variable));
                            return Err(anyhow!(
                                "Secret '{}' has no value. Tried: {}",
                                secret_spec.secret_name,
                                tried.join(", ")
                            ));
                        }
                    },
                }
            }
            DeploymentSecretSource::FilePath(path_str) => {
                let path = Path::new(path_str);
                if !path.exists() {
                    return Err(anyhow!("Secret file not found: {:?}", path_str));
                }
                let content = fs::read_to_string(path).context(format!("Failed to read secret file {:?}", path_str))?;
                let value = trim_secret_value(&content);
                if value.is_empty() {
                    return Err(anyhow!("Secret file {:?} is empty", path_str));
                }
                SecretResolvedValue::Literal(value.to_string())
            }
            DeploymentSecretSource::Embedded(value) => SecretResolvedValue::Literal(value.clone()),
            DeploymentSecretSource::Aws(reference) => resolve_aws_secret(env_spec, reference)?,
        };
        match &value {
            SecretResolvedValue::Literal(literal) => {
                secrets.values.insert(secret_spec.secret_name.clone(), literal.clone());
            }
            SecretResolvedValue::Deferred(_) => {
                secrets.deferred.insert(secret_spec.secret_name.clone());
            }
        }
        secrets.specs.push(SecretResolvedSpec {
            name: format!("{}-{}", app_spec.name, secret_spec.secret_name),
            value,
        });
    }
    Ok(secrets)
}

/// Everything `resolve_service` needs from its surroundings, plus the state
/// shared between the services of one deployment.
struct ServiceResolver<'a> {
    env_spec: &'a DeploymentEnvironmentSpec,
    app_spec: &'a AppSpec,
    deployment: &'a DeploymentSpec,
    configs: &'a [ConfigResolvedSpec],
    secrets: &'a [SecretResolvedSpec],
    deployment_environment: &'a [EnvVariable],
    undockerized_values: &'a [EnvVariable],
    /// (environment, undockerized environment) resolved per host domain. The
    /// full set depends only on the host a service is served from, so it is
    /// computed once per host and every service then picks the entries it asks for.
    env_by_host: HashMap<String, (Vec<EnvVariable>, Vec<EnvVariable>)>,
    /// (host, prefix) pairs claimed by public services so far.
    public_routes: HashSet<(String, String)>,
}

impl ServiceResolver<'_> {
    fn resolve_service(&mut self, app_service: &ServiceSpec) -> Result<ServiceResolvedSpec> {
        let deployment = self.deployment;
        let app_spec = self.app_spec;
        let deployment_service = deployment.services.get(&app_service.name);

        let empty_routes = Vec::new();
        let (variant_name, routes, resources) = match deployment_service {
            Some(ds) => (ds.variant.as_deref().unwrap_or("default"), &ds.routes, &ds.resources),
            None => ("default", &empty_routes, &deployment.defaults),
        };

        // `relative` environment variables resolve against the deployment's
        // primary host, not against wherever a given service happens to be
        // routed. A service can answer on several hosts, so its own routing
        // cannot supply a single base URL; the deployment's does, and it is the
        // same answer for every service in the deployment.
        let host_name = deployment.primary_host.clone();
        let host_domain_name = self
            .env_spec
            .ingress
            .hosts
            .iter()
            .find(|host_spec| host_spec.name == host_name)
            .and_then(|h| h.domain_names.first())
            .ok_or_else(|| anyhow!("Host {} not found in ingress spec", host_name))?;

        let image = self.resolve_image(app_service, variant_name)?;

        // Check Public Service uniqueness
        if let ServiceType::Public = app_service.service_type {
            for route in routes {
                let route_host = route.host.clone().unwrap_or_else(|| deployment.primary_host.clone());
                // Each route's alias must exist, or the service would simply be
                // left out of the generated ingress and silently unreachable.
                // The relative-env lookup below no longer covers this: it
                // resolves the deployment's primary host, not the service's.
                if !self
                    .env_spec
                    .ingress
                    .hosts
                    .iter()
                    .any(|host_spec| host_spec.name == route_host)
                {
                    return Err(anyhow!("Host {} not found in ingress spec", route_host));
                }
                for prefix in &route.prefixes {
                    let key = (route_host.clone(), prefix.prefix.clone());
                    if !self.public_routes.insert(key) {
                        return Err(anyhow!(
                            "Duplicate host+prefix combination for public service {}: {}{}",
                            app_service.name,
                            route_host,
                            prefix.prefix
                        ));
                    }
                }
            }
        }

        // Resolve Environment Variables
        let use_tls = self.env_spec.ingress.tls.is_some();
        if !self.env_by_host.contains_key(host_domain_name) {
            let environment =
                resolve_app_env_vars(app_spec, self.deployment_environment, Some(host_domain_name), use_tls)?;
            let undockerized =
                resolve_app_env_vars(app_spec, self.undockerized_values, Some(host_domain_name), use_tls)?;
            self.env_by_host
                .insert(host_domain_name.clone(), (environment, undockerized));
        }
        let (environment_variables, undockerized_variables) = &self.env_by_host[host_domain_name];
        let final_service_env_vars = filter_service_env_vars(app_service, app_spec, environment_variables)?;
        let final_undockerized_service_env_vars =
            filter_service_env_vars(app_service, app_spec, undockerized_variables)?;

        // Resolve Configs
        let mut service_configs = Vec::new();
        for sc_opt in &app_service.configs {
            let config_name = format!("{}-{}", app_spec.name, sc_opt.config_name);
            if !self.configs.iter().any(|c| c.name == config_name) {
                return Err(anyhow!(
                    "Service {} references undefined config {}",
                    app_service.name,
                    config_name
                ));
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
            if !self.secrets.iter().any(|s| s.name == secret_name) {
                return Err(anyhow!(
                    "Service {} references undefined secret {}",
                    app_service.name,
                    secret_name
                ));
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
        if let Some(ds) = deployment_service {
            for volume in &ds.volumes {
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

        Ok(ServiceResolvedSpec {
            full_name: app_service.name.to_string(),
            service_type: app_service.service_type.clone(),
            is_app_service: app_service.is_app_service,
            image,
            environment_variables: final_service_env_vars,
            undockerized_environment_variables: final_undockerized_service_env_vars,
            configs: service_configs,
            secrets: service_secrets,
            volumes: service_volumes,
            // The app declares the ports its image listens on; a deployment
            // may add more for the gateway to reach without publishing them.
            expose: app_service
                .expose
                .iter()
                .chain(deployment_service.iter().flat_map(|ds| ds.expose.iter()))
                .cloned()
                .collect(),
            command: deployment_service
                .and_then(|s| s.command.clone())
                .or_else(|| app_service.command.clone()),
            entrypoint: deployment_service
                .and_then(|s| s.entrypoint.clone())
                .or_else(|| app_service.entrypoint.clone()),
            healthcheck: app_service.healthcheck.clone(),
            depends_on: app_service.depends_on.clone(),
            resources: resources.clone(),
            // A deployment entry that only sets routing (host, prefix, replicas)
            // must not wipe the ports the app spec declares.
            ports: deployment_service
                .map(|s| s.ports.clone())
                .filter(|ports| !ports.is_empty())
                .unwrap_or_else(|| app_service.ports.clone()),
            working_dir: deployment_service.and_then(|s| s.working_dir.clone()),
            excluded: deployment.exclude_services.contains(&app_service.name),
        })
    }

    /// The image a service runs: the selected variant, tagged with the app
    /// version for app services (`latest` locally), and mapped through the
    /// registry table.
    fn resolve_image(&self, app_service: &ServiceSpec, variant_name: &str) -> Result<String> {
        let mut raw_image = match &app_service.image {
            ImageSpec::Exact(img) => img.clone(),
            ImageSpec::Variants(variants) => variants
                .iter()
                .find(|v| v.variant_name == variant_name)
                .map(|v| v.image.clone())
                .ok_or_else(|| {
                    anyhow!(
                        "Image variant '{}' not found for service '{}'",
                        variant_name,
                        app_service.name
                    )
                })?,
        };

        if !app_service.is_app_service {
            return Ok(raw_image);
        }

        if let DeploymentEnvType::Local = self.env_spec.env_type {
            raw_image = format!("{}:latest", raw_image);
        } else {
            raw_image = format!("{}:{}", raw_image, version_to_tag(&self.app_spec.version.to_string()));
        }
        resolve_app_service_image(self.env_spec, raw_image)
    }
}

/// A public service the deployment configures but gives no prefix would never
/// be reachable through the gateway, which is almost certainly a mistake.
fn check_public_services_are_routed(deployment: &DeploymentSpec, app_spec: &AppSpec) -> Result<()> {
    for app_service in app_spec.all_services() {
        if !matches!(app_service.service_type, ServiceType::Public) {
            continue;
        }
        if let Some(ds) = deployment.services.get(&app_service.name) {
            if ds.routes.iter().all(|route| route.prefixes.is_empty()) {
                return Err(anyhow!(
                    "Public service '{}' in deployment '{}' has no prefixes configured and will not be reachable via ingress.",
                    app_service.name,
                    deployment.name
                ));
            }
        }
    }
    Ok(())
}

/// The gateway as every generator sees it: one rule per served domain, the
/// redirects, the TLS settings and the full list of domains a certificate has to
/// cover.
fn resolve_ingress(env_spec: &DeploymentEnvironmentSpec) -> Result<IngressResolvedSpec> {
    let rules = build_ingress_rules(env_spec);
    check_route_conflicts(&rules)?;

    let tls = env_spec.ingress.tls.as_ref().map(|tls_spec| IngressTlsResolvedSpec {
        secret: tls_spec.secret.clone(),
        letsencrypt: tls_spec.letsencrypt.as_ref().map(|le| LetsEncryptResolvedSpec {
            server: le
                .server
                .clone()
                .unwrap_or("https://acme-v02.api.letsencrypt.org/directory".to_string()),
            email: le.email.clone(),
        }),
    });

    let redirects = resolve_redirects(&env_spec.ingress)?;

    // Redirect sources are domains the gateway answers on without routing them to
    // a service, so they belong in `domains` (which drives the certificate) even
    // though they carry no rules.
    let mut domains: Vec<String> = env_spec
        .ingress
        .hosts
        .iter()
        .flat_map(|h| h.domain_names.clone())
        .collect();
    domains.extend(redirects.iter().map(|r| r.from_domain.clone()));

    Ok(IngressResolvedSpec {
        name: env_spec.ingress.name.clone(),
        domains,
        rules,
        redirects,
        tls,
    })
}

/// One rule per served domain, listing every (deployment, service, prefix)
/// routed to it. Every deployment of the env spec takes part: they are deployed
/// side by side and share the gateway.
fn build_ingress_rules(env_spec: &DeploymentEnvironmentSpec) -> Vec<IngressRule> {
    let mut ingress_rules = Vec::new();
    for host_spec in &env_spec.ingress.hosts {
        for domain in &host_spec.domain_names {
            let mut service_rules = Vec::new();

            for dep in &env_spec.deployments {
                // Services live in a HashMap, so sort by name to keep the
                // generated ingress configuration byte-identical across runs.
                let mut dep_services: Vec<_> = dep.services.iter().collect();
                dep_services.sort_by(|a, b| a.0.cmp(b.0));
                for (service_name, ds) in dep_services {
                    // Which port the gateway talks to. `expose` comes first:
                    // it names the container's port without publishing it, so a
                    // service listening on 1337 is routable on a server that
                    // already hosts another deployment of the same app. Then
                    // port 80 when the service publishes it, otherwise its
                    // first published port, otherwise 80.
                    let port = if let Some(exposed) = ds.expose.first().and_then(|p| p.parse::<u16>().ok()) {
                        exposed
                    } else if ds.ports.iter().any(|p| p.external == 80) {
                        80
                    } else {
                        ds.ports.first().map(|p| p.external).unwrap_or(80)
                    };

                    // A service's own limit wins over the gateway-wide
                    // default; resolving it here means every generator
                    // sees one effective number per route.
                    let body_limit = ds.body_limit.or(env_spec.ingress.body_limit);

                    // A service can be served on several aliases, each with its
                    // own prefixes; only the routes naming THIS alias belong in
                    // this host group's rules.
                    for route in &ds.routes {
                        let host = route.host.as_deref().unwrap_or(&dep.primary_host);
                        if host != host_spec.name {
                            continue;
                        }
                        for prefix in &route.prefixes {
                            service_rules.push(IngressToServiceRule {
                                service_name: service_name.clone(),
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
    ingress_rules
}

/// Guard against ambiguous ingress routing: within a single domain, two
/// services mapping to the same path prefix cannot be disambiguated by a
/// host-based ingress (nginx/traefik/k8s) or the local gateway, so one route
/// would silently shadow the other. The same domain can be spread across
/// several rules (declared under multiple host groups), so aggregate the
/// prefixes by domain across all rules. Prefixes are normalized so that "",
/// "/", and a trailing-slash variant all compare equal.
fn check_route_conflicts(rules: &[IngressRule]) -> Result<()> {
    let normalize_prefix = |prefix: &str| -> String {
        if prefix.is_empty() || prefix == "/" {
            "/".to_string()
        } else {
            prefix.trim_end_matches('/').to_string()
        }
    };
    let mut seen_prefixes: HashMap<&str, HashMap<String, (&str, &str)>> = HashMap::new();
    for rule in rules {
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
    Ok(())
}

/// Flattens `gateway.redirects` into one rule per source domain.
///
/// A source that is also declared under `gateway.hosts` would shadow every route
/// on that domain, and a source declared twice has no defined winner, so both are
/// rejected here rather than silently resolved by whichever generator runs.
fn resolve_redirects(ingress: &IngressSpec) -> Result<Vec<RedirectRule>> {
    let served_domains: Vec<&str> = ingress
        .hosts
        .iter()
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
                    from,
                    previous.to,
                    redirect.to
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
                return Err(anyhow!(
                    "Docker registry host for namespace '{}' not found in environment spec. Available namespaces: {:?}",
                    namespace,
                    available
                ));
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
fn resolve_secret_refs(input: &str, secrets: &HashMap<String, String>, deferred: &HashSet<String>) -> Result<String> {
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
                None if deferred.contains(secret_name) => {
                    return Err(anyhow!(
                        "$secret({}) cannot be used: the secret has an 'aws' source, so its value is \
                     only fetched on the deploy target and cannot be substituted into an env \
                     variable here. Mount it on the service with `variable:` instead.",
                        secret_name
                    ))
                }
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
        let val = deployment_values
            .iter()
            .find(|e| e.name == external.name)
            .map(|e| e.value.clone())
            .or_else(|| external.default.clone());

        if let Some(v) = val {
            add_unique_var(
                &mut environment_variables,
                EnvVariable {
                    name: external.name.clone(),
                    value: v,
                },
            );
        } else {
            return Err(anyhow!("Missing external env variable: {}", external.name));
        }
    }

    // Optional
    for optional in &app_spec.environment.optional {
        let val = deployment_values
            .iter()
            .find(|e| e.name == optional.name)
            .map(|e| e.value.clone());

        if let Some(v) = val {
            add_unique_var(
                &mut environment_variables,
                EnvVariable {
                    name: optional.name.clone(),
                    value: v,
                },
            );
        }
    }

    // Relative
    for relative in &app_spec.environment.relative {
        if let Some(h) = host_domain_name {
            let scheme = if use_tls { "https" } else { "http" };
            let url = format!("{}://{}{}", scheme, h, relative.relative_value);
            let value = resolve_variable_in_string(&url, &environment_variables)
                .context(format!("Failed to resolve relative env variable {}", relative.name))?;
            add_unique_var(
                &mut environment_variables,
                EnvVariable {
                    name: relative.name.clone(),
                    value,
                },
            );
        }
    }

    // Internal
    for internal in &app_spec.environment.internal {
        let value = resolve_variable_in_string(&internal.value, &environment_variables)
            .context(format!("Failed to resolve internal env variable {}", internal.name))?;
        add_unique_var(
            &mut environment_variables,
            EnvVariable {
                name: internal.name.clone(),
                value,
            },
        );
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
    all_env_vars: &[EnvVariable],
) -> Result<Vec<EnvVariable>> {
    // Optional variables this environment did not provide. A service may reference
    // one either directly or through `${...}`; the entry is then left off the
    // service instead of failing the deployment, which is what makes it optional.
    let unset_optional: HashSet<&str> = app_spec
        .environment
        .optional
        .iter()
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
                    return Err(anyhow!(
                        "Service {} references undefined env var {}",
                        app_service.name,
                        name
                    ));
                }
            }
            ServiceEnvOption::WithValue(k, v) => {
                if referenced_var_names(v).iter().any(|n| unset_optional.contains(n)) {
                    continue;
                }
                add_unique_var(
                    &mut final_service_env_vars,
                    EnvVariable {
                        name: k.clone(),
                        value: resolve_variable_in_string(v, all_env_vars)
                            .context(format!("{}: Failed to resolve env var {}={}", app_service.name, k, v))?,
                    },
                );
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
            hosts: hosts
                .iter()
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
                optional: optional
                    .iter()
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
        assert!(
            vars.is_empty(),
            "unset optional vars must not reach the service: {:?}",
            vars
        );
    }

    #[test]
    fn provided_optional_var_reaches_the_service() {
        let app_spec = optional_app_spec(&["LIVEKIT_URL", "LIVEKIT_API_KEY"]);
        let service = service_with_env(vec![
            ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string()),
            ServiceEnvOption::WithValue("URL".to_string(), "${LIVEKIT_URL}".to_string()),
        ]);
        let all = vec![
            EnvVariable {
                name: "LIVEKIT_API_KEY".to_string(),
                value: "key".to_string(),
            },
            EnvVariable {
                name: "LIVEKIT_URL".to_string(),
                value: "wss://lk".to_string(),
            },
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

    mod full_resolve {
        //! `resolve` on specs written as YAML: the happy path once, then every
        //! way a spec can be rejected during resolution.
        use super::*;
        use crate::test_support::{app_spec, env_spec, try_env_spec};

        const APP: &str = r#"
name: shop
version: 1.2.3
environment:
  external:
    - LOG_LEVEL=info
  relative:
    - PUBLIC_URL=/
app_services:
  api:
    type: public
    image: myorg/api
    environment:
      - $all
    ports:
      - "80:8080"
extra_services:
  primary-db:
    type: internal
    image: postgres:16
"#;

        fn env(env_type: &str, registry: &str, prod_services: &str, extra_deployments: &str) -> String {
            format!(
                r#"
type: {env_type}
gateway:
  hosts:
    web: shop.example.com
    admin: admin.example.com
  tls:
    disable: true
{registry}
deployments:
  prod:
    primary_host: web
    application:
      name: shop
    services:
{prod_services}
{extra_deployments}
"#
            )
        }

        const REGISTRY: &str = "registry:\n  myorg: registry.example.com";
        const API_ROUTED: &str = "      api:\n        host: web\n        prefix: /";

        fn resolve_yaml(app: &str, env_yaml: &str) -> Result<EnvironmentResolvedSpec> {
            let root = tempfile::tempdir().unwrap();
            resolve(&env_spec(env_yaml, root.path()), &app_spec(app), "prod")
        }

        fn error(app: &str, env_yaml: &str) -> String {
            resolve_yaml(app, env_yaml).unwrap_err().to_string()
        }

        /// Errors raised while *converting* the env spec, before resolution —
        /// `resolve_yaml` cannot reach these, since its helper unwraps first.
        fn convert_error(env_yaml: &str) -> String {
            let root = tempfile::tempdir().unwrap();
            try_env_spec(env_yaml, root.path()).unwrap_err().to_string()
        }

        #[test]
        fn a_kubernetes_deployment_resolves_images_and_environment() {
            let spec = resolve_yaml(APP, &env("k8s", REGISTRY, API_ROUTED, "")).unwrap();
            let api = spec
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "api")
                .unwrap();
            let db = spec
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "primary-db")
                .unwrap();

            // App images get the version and the registry; extra images are untouched.
            assert_eq!(api.image, "registry.example.com/myorg/api:1.2.3");
            assert_eq!(db.image, "postgres:16");

            let value = |name: &str| {
                api.environment_variables
                    .iter()
                    .find(|v| v.name == name)
                    .map(|v| v.value.as_str())
            };
            assert_eq!(value("LOG_LEVEL"), Some("info"));
            // No TLS, so relative URLs are http on the service's host.
            assert_eq!(value("PUBLIC_URL"), Some("http://shop.example.com/"));

            assert_eq!(spec.ingress.rules.len(), 1);
            assert_eq!(spec.ingress.rules[0].domain_name, "shop.example.com");
            assert_eq!(spec.ingress.rules[0].services[0].service_name, "api");
        }

        #[test]
        fn a_local_deployment_uses_the_latest_local_image() {
            let local = r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  prod:
    primary_host: web
    application:
      name: shop
    services:
      api:
        host: web
        prefix: /
        ports:
          - "8080:80"
"#;
            let spec = resolve_yaml(APP, local).unwrap();
            let api = spec
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "api")
                .unwrap();
            assert_eq!(api.image, "myorg/api:latest");
        }

        #[test]
        fn a_missing_registry_namespace_is_rejected() {
            let err = error(APP, &env("k8s", "registry:\n  other: r.example.com", API_ROUTED, ""));
            assert!(err.contains("namespace 'myorg' not found"), "{err}");

            let err = error(APP, &env("k8s", "", API_ROUTED, ""));
            assert!(err.contains("Registry mapping is required"), "{err}");
        }

        #[test]
        fn an_unknown_image_variant_is_rejected() {
            let app = APP.replace(
                "    image: myorg/api\n",
                "    variants:\n      arm:\n        image: myorg/api-arm\n",
            );
            let services = "      api:\n        host: web\n        prefix: /\n        variant: x86";
            let err = error(&app, &env("k8s", REGISTRY, services, ""));
            assert!(err.contains("Image variant 'x86' not found for service 'api'"), "{err}");
        }

        #[test]
        fn an_unknown_host_is_rejected() {
            let services = "      api:\n        host: nope\n        prefix: /";
            let err = error(APP, &env("k8s", REGISTRY, services, ""));
            assert!(err.contains("Host nope not found in ingress spec"), "{err}");
        }

        #[test]
        fn a_service_is_routed_on_every_host_it_names() {
            // The case this exists for: a CMS on its own admin domain, plus a
            // prefix on the site's domain so uploads stay same-origin.
            let services = "      api:
        hosts:
          admin:
            prefix: /
            strip_prefix: false
          web:
            prefixes:
              \"/upload\":
                strip: false";
            let resolved = resolve_yaml(APP, &env("k8s", REGISTRY, services, "")).unwrap();

            let rule = |domain: &str| {
                resolved
                    .ingress
                    .rules
                    .iter()
                    .find(|r| r.domain_name == domain)
                    .unwrap_or_else(|| panic!("no rule for {domain}"))
            };

            let admin = rule("admin.example.com");
            assert_eq!(admin.services.len(), 1);
            assert_eq!(admin.services[0].service_name, "api");
            assert_eq!(admin.services[0].prefix, "/");

            // The site's domain keeps only what it was given: the "/" route did
            // not leak across from the admin host.
            let site = rule("shop.example.com");
            assert_eq!(site.services.len(), 1, "unexpected routes: {:?}", site.services);
            assert_eq!(site.services[0].service_name, "api");
            assert_eq!(site.services[0].prefix, "/upload");
        }

        #[test]
        fn expose_routes_the_gateway_without_publishing_a_host_port() {
            let services = "      api:
        host: web
        prefix: /
        expose:
          - \"1337\"";
            let resolved = resolve_yaml(APP, &env("k8s", REGISTRY, services, "")).unwrap();

            let route = &resolved
                .ingress
                .rules
                .iter()
                .find(|r| r.domain_name == "shop.example.com")
                .unwrap()
                .services[0];
            assert_eq!(route.port, 1337, "gateway should talk to the exposed port");

            let api = resolved
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "api")
                .unwrap();
            // The deployment's `expose` adds no published port: the service
            // still publishes only what the app spec declared (80:8080).
            assert_eq!(api.ports.len(), 1);
            assert_eq!(api.ports[0].external, 80);
            assert!(api.expose.contains(&"1337".to_string()));
        }

        #[test]
        fn mixing_hosts_with_the_single_host_form_is_rejected() {
            let services = "      api:
        host: web
        prefix: /
        hosts:
          web:
            prefix: /";
            let err = convert_error(&env("k8s", REGISTRY, services, ""));
            assert!(err.contains("both `hosts` and the single-host"), "{err}");
        }

        #[test]
        fn a_host_named_without_a_prefix_is_rejected() {
            let services = "      api:
        hosts:
          web: {}";
            let err = convert_error(&env("k8s", REGISTRY, services, ""));
            assert!(err.contains("without a `prefix` or `prefixes`"), "{err}");
        }

        #[test]
        fn an_unknown_host_among_several_is_rejected() {
            let services = "      api:
        hosts:
          web:
            prefix: /
          nope:
            prefix: /x";
            let err = error(APP, &env("k8s", REGISTRY, services, ""));
            assert!(err.contains("Host nope not found in ingress spec"), "{err}");
        }

        #[test]
        fn an_unknown_field_on_a_service_is_rejected() {
            // `hosts` used to land here: unknown keys parsed and were dropped,
            // so a typo produced wrong routing with no error at all.
            let services = "      api:
        host: web
        prefix: /
        prefixxes:
          \"/x\": {}";
            let err = convert_error(&env("k8s", REGISTRY, services, ""));
            assert!(err.contains("prefixxes"), "{err}");
        }

        #[test]
        fn a_public_service_without_a_prefix_is_rejected() {
            let services = "      api:\n        host: web";
            let err = error(APP, &env("k8s", REGISTRY, services, ""));
            assert!(
                err.contains("Public service 'api'") && err.contains("no prefixes"),
                "{err}"
            );
        }

        #[test]
        fn two_public_services_cannot_share_a_host_and_prefix() {
            let app = format!("{APP}  admin:\n    type: public\n    image: myorg/admin\n");
            let services =
                "      api:\n        host: web\n        prefix: /\n      admin:\n        host: web\n        prefix: /";
            let err = error(&app, &env("k8s", REGISTRY, services, ""));
            assert!(err.contains("Duplicate host+prefix"), "{err}");
        }

        #[test]
        fn two_deployments_cannot_route_the_same_domain_and_path() {
            let staging = "  staging:\n    primary_host: web\n    application:\n      name: shop\n    services:\n      api:\n        host: web\n        prefix: /";
            let err = error(APP, &env("k8s", REGISTRY, API_ROUTED, staging));
            assert!(err.contains("maps path '/' to multiple services"), "{err}");
        }

        #[test]
        fn a_deployment_volume_must_be_declared_by_the_app() {
            let services =
                "      api:\n        host: web\n        prefix: /\n        volumes:\n          - cache:/cache";
            let err = error(APP, &env("k8s", REGISTRY, services, ""));
            assert!(
                err.contains("named volume 'cache'") && err.contains("not declared"),
                "{err}"
            );

            let app = format!("{APP}volumes:\n  - cache\n");
            let spec = resolve_yaml(&app, &env("k8s", REGISTRY, services, "")).unwrap();
            let api = spec
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "api")
                .unwrap();
            assert_eq!(api.volumes.len(), 1);
        }

        #[test]
        fn a_secret_from_an_unset_or_empty_environment_variable_is_rejected() {
            let app = format!("{APP}secrets:\n  - db_password\n");
            let secrets = |var: &str| {
                format!("      api:\n        host: web\n        prefix: /\n    secrets:\n      db_password:\n        env: {var}")
            };
            // `services:` is closed by the `secrets:` key at deployment level.
            let err = error(&app, &env("k8s", REGISTRY, &secrets("SIMPLED_TEST_UNSET_SECRET"), ""));
            assert!(err.contains("SIMPLED_TEST_UNSET_SECRET not set"), "{err}");

            std::env::set_var("SIMPLED_TEST_EMPTY_SECRET", "");
            let err = error(&app, &env("k8s", REGISTRY, &secrets("SIMPLED_TEST_EMPTY_SECRET"), ""));
            assert!(err.contains("SIMPLED_TEST_EMPTY_SECRET is empty"), "{err}");
        }

        /// A secret file is written by a person or by CI, and both end it with a
        /// newline that is not part of the value — one that a secret mounted as an
        /// environment variable could not carry at all.
        #[test]
        fn a_secret_file_loses_its_trailing_newline_and_cannot_be_empty() {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join("db"), "s3cr3t\n").unwrap();
            fs::write(root.path().join("blank"), "\n").unwrap();

            let app = format!("{APP}secrets:\n  - db_password\n");
            let services = |file: &str| {
                format!("      api:\n        host: web\n        prefix: /\n    secrets:\n      db_password:\n        file: {file}")
            };
            let resolve_file = |file: &str| {
                resolve(
                    &env_spec(&env("k8s", REGISTRY, &services(file), ""), root.path()),
                    &app_spec(&app),
                    "prod",
                )
            };

            let spec = resolve_file("db").unwrap();
            assert_eq!(spec.current_deployment.secrets[0].literal(), Some("s3cr3t"));

            let err = resolve_file("blank").unwrap_err().to_string();
            assert!(err.contains("is empty"), "{err}");
        }

        /// The variable's name is spelled from the secret's, and the two follow
        /// different conventions, so the match ignores case.
        #[test]
        fn a_prefixed_secret_is_read_from_the_environment_whatever_case_it_is_in() {
            let app = format!("{APP}secrets:\n  - db_password\n");
            let prefixed = |prefix: &str| {
                format!("      api:\n        host: web\n        prefix: /\n    secrets_env_prefix: {prefix}\n    secrets:\n      db_password:")
            };
            let resolve_prefix = |prefix: &str| resolve_yaml(&app, &env("k8s", REGISTRY, &prefixed(prefix), ""));

            std::env::set_var("SIMPLED_TEST_PFX_DB_PASSWORD", "s3cr3t\n");
            let spec = resolve_prefix("SIMPLED_TEST_PFX_").unwrap();
            assert_eq!(spec.current_deployment.secrets[0].literal(), Some("s3cr3t"));

            let err = resolve_prefix("SIMPLED_TEST_UNSET_PFX_").unwrap_err().to_string();
            assert!(err.contains("$SIMPLED_TEST_UNSET_PFX_db_password (not set)"), "{err}");

            std::env::set_var("SIMPLED_TEST_BLANK_PFX_DB_PASSWORD", "\n");
            let err = resolve_prefix("SIMPLED_TEST_BLANK_PFX_").unwrap_err().to_string();
            assert!(err.contains("is empty"), "{err}");
        }

        /// `secrets_aws` ends the chain. It is the one source that cannot be tried
        /// while the deployment is prepared, so an unset variable defers to it.
        #[test]
        fn an_unset_prefixed_variable_falls_through_to_secrets_aws() {
            let app = format!("{APP}secrets:\n  - db_password\n");
            let services = "      api:\n        host: web\n        prefix: /\n    secrets_env_prefix: SIMPLED_TEST_NO_SUCH_PREFIX_\n    secrets_aws: prod/shop/bundle\n    secrets:\n      db_password:";
            let spec = resolve_yaml(&app, &env("k8s", REGISTRY, services, "")).unwrap();
            let deferred = spec.current_deployment.secrets[0].deferred().unwrap();
            assert_eq!(deferred.secret_id, "prod/shop/bundle");
            assert_eq!(deferred.jq.as_deref(), Some(".db_password"));
        }

        #[test]
        fn a_secret_reference_to_a_deferred_secret_is_rejected() {
            let app = format!("{APP}secrets:\n  - db_password\n");
            let services = "      api:\n        host: web\n        prefix: /\n    environment:\n      - DB_URL=postgres://u:$secret(db_password)@db\n    secrets:\n      db_password:\n        aws: prod/shop/db";
            let err = error(&app, &env("k8s", REGISTRY, services, ""));
            assert!(err.contains("$secret(db_password) cannot be used"), "{err}");
        }

        #[test]
        fn a_literal_secret_can_be_referenced_from_the_environment() {
            let app = APP.replace("    - LOG_LEVEL=info\n", "    - LOG_LEVEL=info\n    - DB_URL\n")
                + "secrets:\n  - db_password\n";
            let services = "      api:\n        host: web\n        prefix: /\n    environment:\n      - DB_URL=postgres://u:$secret(db_password)@db\n    secrets:\n      db_password: s3cr3t";
            let spec = resolve_yaml(&app, &env("k8s", REGISTRY, services, "")).unwrap();
            let api = spec
                .current_deployment
                .services
                .iter()
                .find(|s| s.full_name == "api")
                .unwrap();
            let db_url = api.environment_variables.iter().find(|v| v.name == "DB_URL").unwrap();
            assert_eq!(db_url.value, "postgres://u:s3cr3t@db");
            assert_eq!(spec.current_deployment.secrets[0].name, "shop-db_password");
            assert_eq!(spec.current_deployment.secrets[0].literal(), Some("s3cr3t"));
        }
    }
}
