use crate::spec::*;
use crate::spec_yaml::*;
use crate::{env_loader, spec};
use anyhow::{Context, Result, anyhow};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

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

    // Resolve `extends` inheritance before anything inspects the deployments, so
    // every downstream check (secrets_folder, working_dir, ...) sees the fully
    // merged spec. Abstract deployments are templates only and are dropped here.
    let resolved = resolve_extends(&yaml.deployments)?;
    let mut concrete: HashMap<String, DeploymentSpecYaml> = resolved
        .into_iter()
        .filter(|(_, dep)| dep.is_abstract != Some(true))
        .collect();

    // In a local environment only one deployment runs at a time, so once the caller
    // has picked one the others take no part in this run. Dropping them here keeps
    // checks that span deployments (ingress routing above all) from reporting
    // conflicts against a deployment that is never going to start — sibling
    // deployments normally claim the same domains and ports on purpose. Every other
    // environment type deploys its deployments side by side, so there the full set
    // is kept and the conflicts are real.
    if matches!(env_type_yaml, DeploymentEnvTypeYaml::Local) {
        if let Some(selected) = selected_deployment {
            if !concrete.contains_key(selected) {
                let mut available: Vec<&str> = concrete.keys().map(|s| s.as_str()).collect();
                available.sort();
                return Err(anyhow!(
                    "Deployment '{}' not found in env spec. Available deployments: {}",
                    selected,
                    available.join(", ")
                ));
            }
            concrete.retain(|name, _| name == selected);
        }
    }

    let mut deployments = Vec::new();
    for (name, dep) in &concrete {
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
            if concrete.values().any(|d| d.secrets_folder.is_some()) {
                return Err(anyhow!("secrets_folder cannot be set for K8S environment"));
            }
            if any_service_has_working_dir(&concrete) {
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
            if concrete.values().any(|d| d.secrets_folder.is_some()) {
                return Err(anyhow!("secrets_folder cannot be set for Docker environment"));
            }
            if any_service_has_working_dir(&concrete) {
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

            // Port and working_dir uniqueness are checked per deployment: different
            // deployments may reuse the same external ports and directories since only
            // one runs at a time. Services are visited in name order so a conflict is
            // always reported with the same pair of names.
            for dep in &deployments {
                let mut ports_seen = HashSet::new();
                let mut working_dirs_seen: HashMap<PathBuf, &str> = HashMap::new();
                if let Some(services) = &dep.services {
                    let mut svc_names: Vec<&String> = services.keys().collect();
                    svc_names.sort();
                    for svc_name in svc_names {
                        let svc_spec = &services[svc_name];
                        if svc_spec.ports.is_empty() {
                            return Err(anyhow!("In Local environment, service {} must have at least one port", svc_name));
                        }
                        for port in &svc_spec.ports {
                            if !ports_seen.insert(port.external) {
                                return Err(anyhow!("Duplicate external port {} in deployment {}", port.external, dep.name));
                            }
                        }
                        // A host-run service writes its `.env` and secret files straight
                        // into `working_dir`, so two services pointing at the same
                        // directory would silently overwrite each other's files.
                        if let Some(dir) = &svc_spec.working_dir {
                            if let Some(other) = working_dirs_seen.insert(normalize_working_dir(dir), svc_name) {
                                return Err(anyhow!(
                                    "Services '{}' and '{}' in deployment '{}' share working_dir '{}'. \
                                     Each host-run service needs its own directory: its .env file and \
                                     secrets are written there and would overwrite each other.",
                                    other, svc_name, dep.name, dir
                                ));
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

/// Resolves `extends` inheritance for every deployment, returning a map of fully
/// merged specs keyed by the original names. Chains of `extends` are followed and
/// cycles are reported as errors.
fn resolve_extends(
    raw: &HashMap<String, DeploymentSpecYaml>,
) -> Result<HashMap<String, DeploymentSpecYaml>> {
    let mut resolved: HashMap<String, DeploymentSpecYaml> = HashMap::new();
    for name in raw.keys() {
        resolve_deployment(name, raw, &mut resolved, &mut Vec::new())?;
    }
    Ok(resolved)
}

fn resolve_deployment(
    name: &str,
    raw: &HashMap<String, DeploymentSpecYaml>,
    resolved: &mut HashMap<String, DeploymentSpecYaml>,
    stack: &mut Vec<String>,
) -> Result<DeploymentSpecYaml> {
    if let Some(done) = resolved.get(name) {
        return Ok(done.clone());
    }
    if stack.iter().any(|n| n == name) {
        stack.push(name.to_string());
        return Err(anyhow!("Cyclic 'extends' chain in deployments: {}", stack.join(" -> ")));
    }

    let dep = raw
        .get(name)
        .ok_or_else(|| anyhow!("Deployment '{}' extends unknown deployment '{}'", stack.last().map(|s| s.as_str()).unwrap_or(name), name))?;

    let merged = match &dep.extends {
        Some(base_name) => {
            stack.push(name.to_string());
            let base = resolve_deployment(base_name, raw, resolved, stack)?;
            stack.pop();
            let mut merged = merge_deployment(&base, dep);
            // The merged spec is standalone; keeping `extends` would be misleading.
            merged.extends = None;
            merged
        }
        None => dep.clone(),
    };

    resolved.insert(name.to_string(), merged.clone());
    Ok(merged)
}

/// Merges a child deployment onto its already-resolved base. Scalar fields fall
/// back to the base when the child leaves them unset; map fields and environment
/// lists are unioned with the child's entries winning on key conflicts.
fn merge_deployment(base: &DeploymentSpecYaml, child: &DeploymentSpecYaml) -> DeploymentSpecYaml {
    DeploymentSpecYaml {
        extends: child.extends.clone(),
        // Abstractness is a property of the deployment itself, never inherited.
        is_abstract: child.is_abstract,
        primary_host: child.primary_host.clone().or_else(|| base.primary_host.clone()),
        application: merge_application(base.application.as_ref(), child.application.as_ref()),
        environment: merge_env_variables(base.environment.as_ref(), child.environment.as_ref()),
        undockerized_environment: merge_env_variables(
            base.undockerized_environment.as_ref(),
            child.undockerized_environment.as_ref(),
        ),
        configs: merge_opt_map(base.configs.as_ref(), child.configs.as_ref(), |_, c| c.clone()),
        secrets: merge_opt_map(base.secrets.as_ref(), child.secrets.as_ref(), |_, c| c.clone()),
        defaults: child.defaults.clone().or_else(|| base.defaults.clone()),
        services: merge_opt_map(base.services.as_ref(), child.services.as_ref(), merge_service),
        secrets_folder: child.secrets_folder.clone().or_else(|| base.secrets_folder.clone()),
    }
}

/// Merges a child's environment onto the base's, keyed by variable name: the
/// base order is preserved, a child entry redefining a base variable replaces it
/// in place, and new variables are appended. A list and an `.env` file path
/// cannot be combined, so in that case the child replaces the base outright.
fn merge_env_variables(
    base: Option<&DeploymentEnvVariablesYaml>,
    child: Option<&DeploymentEnvVariablesYaml>,
) -> Option<DeploymentEnvVariablesYaml> {
    match (base, child) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(c)) => Some(c.clone()),
        (
            Some(DeploymentEnvVariablesYaml::FromList(b)),
            Some(DeploymentEnvVariablesYaml::FromList(c)),
        ) => {
            let mut out = b.clone();
            let mut index: HashMap<String, usize> = HashMap::new();
            for (i, entry) in out.iter().enumerate() {
                if let Some(name) = env_entry_name(entry) {
                    index.insert(name, i);
                }
            }
            for entry in c {
                let name = env_entry_name(entry);
                match name.as_ref().and_then(|n| index.get(n)).copied() {
                    Some(pos) => out[pos] = entry.clone(),
                    None => {
                        if let Some(n) = name {
                            index.insert(n, out.len());
                        }
                        out.push(entry.clone());
                    }
                }
            }
            Some(DeploymentEnvVariablesYaml::FromList(out))
        }
        (Some(_), Some(c)) => Some(c.clone()),
    }
}

/// Name of the variable an entry defines, or `None` when it cannot be
/// determined. Malformed entries are reported later, when the entry is
/// converted; here they simply never match a base entry.
fn env_entry_name(entry: &EnvVariableEntryYaml) -> Option<String> {
    match entry {
        EnvVariableEntryYaml::Inline(s) => env_loader::parse_env_string(s).ok().map(|d| d.name),
        EnvVariableEntryYaml::FromFile(map) => match map.len() {
            1 => map.keys().next().cloned(),
            _ => None,
        },
    }
}

/// Unions two optional maps. Keys present in both are combined with `combine`,
/// which receives the base and child values in that order.
fn merge_opt_map<V: Clone>(
    base: Option<&HashMap<String, V>>,
    child: Option<&HashMap<String, V>>,
    combine: impl Fn(&V, &V) -> V,
) -> Option<HashMap<String, V>> {
    match (base, child) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(c)) => Some(c.clone()),
        (Some(b), Some(c)) => {
            let mut out = b.clone();
            for (k, cv) in c {
                let merged = match out.get(k) {
                    Some(bv) => combine(bv, cv),
                    None => cv.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Some(out)
        }
    }
}

fn merge_application(
    base: Option<&DeploymentAppSpecYaml>,
    child: Option<&DeploymentAppSpecYaml>,
) -> Option<DeploymentAppSpecYaml> {
    match (base, child) {
        (None, None) => None,
        (Some(b), None) => Some(b.clone()),
        (None, Some(c)) => Some(c.clone()),
        (Some(b), Some(c)) => Some(DeploymentAppSpecYaml {
            name: c.name.clone(),
            version: c.version.clone().or_else(|| b.version.clone()),
            extra: match (&b.extra, &c.extra) {
                (Some(be), Some(ce)) => {
                    let mut v = be.clone();
                    v.extend(ce.clone());
                    Some(v)
                }
                (Some(be), None) => Some(be.clone()),
                (None, ce) => ce.clone(),
            },
        }),
    }
}

/// Field-wise merge of a per-service override; the child wins on any field it sets.
fn merge_service(
    base: &DeploymentServiceSpecYaml,
    child: &DeploymentServiceSpecYaml,
) -> DeploymentServiceSpecYaml {
    DeploymentServiceSpecYaml {
        variant: child.variant.clone().or_else(|| base.variant.clone()),
        host: child.host.clone().or_else(|| base.host.clone()),
        prefix: child.prefix.clone().or_else(|| base.prefix.clone()),
        strip_prefix: child.strip_prefix.or(base.strip_prefix),
        prefixes: merge_opt_map(base.prefixes.as_ref(), child.prefixes.as_ref(), |_, c| c.clone()),
        replicas: child.replicas.or(base.replicas),
        resources: child.resources.clone().or_else(|| base.resources.clone()),
        ports: child.ports.clone().or_else(|| base.ports.clone()),
        working_dir: child.working_dir.clone().or_else(|| base.working_dir.clone()),
    }
}

/// Normalizes a `working_dir` for comparison, so that `./a/b`, `a/b` and `a/b/`
/// are recognised as the same directory. `Path::components` already drops
/// interior `.` segments and trailing separators; the leading `.` it keeps is
/// filtered out here.
fn normalize_working_dir(dir: &str) -> PathBuf {
    Path::new(dir)
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
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
    let primary_host = yaml.primary_host.clone().ok_or_else(|| {
        anyhow!("Deployment '{}' is missing required field 'primary_host'", name)
    })?;
    let application_yaml = yaml.application.as_ref().ok_or_else(|| {
        anyhow!("Deployment '{}' is missing required field 'application'", name)
    })?;
    let application = convert_deployment_app(application_yaml)?;
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
                    let sources = [v.env.is_some(), v.file.is_some(), v.aws.is_some()]
                        .iter()
                        .filter(|present| **present)
                        .count();
                    if sources > 1 {
                        return Err(anyhow!("Secret {} can only have one of the env, file and aws sources", k));
                    }
                    if v.jq.is_some() && v.aws.is_none() {
                        return Err(anyhow!("Secret {} sets jq, which is only valid together with an aws source", k));
                    }
                    let source = if let Some(env) = &v.env {
                        DeploymentSecretSource::EnvVariable(env.clone())
                    } else if let Some(file) = &v.file {
                        DeploymentSecretSource::FilePath(file.clone())
                    } else if let Some(aws) = &v.aws {
                        DeploymentSecretSource::Aws(AwsSecretRef {
                            secret_id: aws.clone(),
                            jq: v.jq.clone(),
                        })
                    } else {
                        return Err(anyhow!("Secret {} must have an env, file or aws source", k));
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
        primary_host,
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

    fn local_env_with_working_dirs(api_dir: &str, worker_dir: &str) -> DeploymentEnvironmentSpecYaml {
        let raw = format!(
            r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  app_local:
    primary_host: web
    application:
      name: app
    services:
      api:
        host: web
        prefix: /
        ports:
          - "8080:80"
        working_dir: {api_dir}
      worker:
        host: web
        ports:
          - "8081:80"
        working_dir: {worker_dir}
"#
        );
        serde_yaml::from_str(&raw).unwrap()
    }

    #[test]
    fn duplicate_working_dir_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let err = convert_env_spec(local_env_with_working_dirs("./backend", "./backend"), root.path(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("share working_dir"), "unexpected error: {err}");
        // Both service names are named, in a stable order.
        assert!(err.contains("'api' and 'worker'"), "unexpected error: {err}");
    }

    #[test]
    fn working_dir_comparison_ignores_path_noise() {
        // `./backend` and `backend/` are the same directory.
        let root = tempfile::tempdir().unwrap();
        let err = convert_env_spec(local_env_with_working_dirs("./backend", "backend/"), root.path(), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("share working_dir"), "unexpected error: {err}");
    }

    #[test]
    fn distinct_working_dirs_are_accepted() {
        let root = tempfile::tempdir().unwrap();
        let spec = convert_env_spec(
            local_env_with_working_dirs("./backend/api", "./backend/worker"),
            root.path(),
            None,
        )
        .unwrap();
        let services = spec.deployments[0].services.as_ref().unwrap();
        assert_eq!(services["api"].working_dir.as_deref(), Some("./backend/api"));
        assert_eq!(services["worker"].working_dir.as_deref(), Some("./backend/worker"));
    }

    fn deployments_map(raw: &str) -> HashMap<String, DeploymentSpecYaml> {
        serde_yaml::from_str(raw).unwrap()
    }

    #[test]
    fn extends_inherits_scalars_and_merges_maps() {
        let raw = r#"
base:
  abstract: true
  primary_host: web
  application:
    name: app
    version: ^1.0.0
  configs:
    shared: ./shared
  services:
    api:
      host: web
      prefix: /
      replicas: 2
child:
  extends: base
  configs:
    child_only: ./child
  services:
    api:
      replicas: 5
    worker:
      host: web
"#;
        let resolved = resolve_extends(&deployments_map(raw)).unwrap();
        let child = &resolved["child"];

        // Scalar fields fall back to the base.
        assert_eq!(child.primary_host.as_deref(), Some("web"));
        assert_eq!(child.application.as_ref().unwrap().name, "app");

        // Map fields are unioned.
        let configs = child.configs.as_ref().unwrap();
        assert_eq!(configs.len(), 2);
        assert!(configs.contains_key("shared") && configs.contains_key("child_only"));

        // Services merge field-wise on key conflicts and add new keys.
        let services = child.services.as_ref().unwrap();
        let api = &services["api"];
        assert_eq!(api.host.as_deref(), Some("web")); // inherited from base
        assert_eq!(api.replicas, Some(5)); // overridden by child
        assert!(services.contains_key("worker"));

        // The resolved spec is standalone.
        assert!(child.extends.is_none());
    }

    #[test]
    fn extends_merges_environment_per_variable() {
        let raw = r#"
base:
  abstract: true
  primary_host: web
  application:
    name: app
  environment:
    - DB=postgres://base
    - REGION=us-east-2
    - KEEP=1
  undockerized_environment:
    - DB=postgres://localhost
    - KEEP=1
child:
  extends: base
  environment:
    - DB=postgres://child
    - EXTRA=yes
  undockerized_environment:
    - EXTRA=yes
"#;
        let resolved = resolve_extends(&deployments_map(raw)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let env = convert_env_variables(&resolved["child"].environment, root.path()).unwrap();

        // Base order is kept, the child's redefinition replaces in place, and new
        // variables are appended.
        let pairs: Vec<(&str, &str)> =
            env.iter().map(|v| (v.name.as_str(), v.value.as_str())).collect();
        assert_eq!(
            pairs,
            vec![
                ("DB", "postgres://child"),
                ("REGION", "us-east-2"),
                ("KEEP", "1"),
                ("EXTRA", "yes"),
            ]
        );

        // undockerized_environment merges the same way.
        let undockerized =
            convert_env_variables(&resolved["child"].undockerized_environment, root.path()).unwrap();
        let names: Vec<&str> = undockerized.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, vec!["DB", "KEEP", "EXTRA"]);
    }

    #[test]
    fn extends_env_file_replaces_inherited_list() {
        let raw = r#"
base:
  abstract: true
  environment:
    - DB=postgres://base
child:
  extends: base
  environment: ./child.env
"#;
        let resolved = resolve_extends(&deployments_map(raw)).unwrap();
        match resolved["child"].environment.as_ref().unwrap() {
            DeploymentEnvVariablesYaml::FromEnvFile(path) => assert_eq!(path, "./child.env"),
            other => panic!("unexpected environment: {other:?}"),
        }
    }

    #[test]
    fn extends_chain_is_followed() {
        let raw = r#"
grandparent:
  abstract: true
  primary_host: web
  application:
    name: app
parent:
  abstract: true
  extends: grandparent
  defaults:
    replicas: 4
child:
  extends: parent
"#;
        let resolved = resolve_extends(&deployments_map(raw)).unwrap();
        let child = &resolved["child"];
        assert_eq!(child.primary_host.as_deref(), Some("web"));
        assert_eq!(child.application.as_ref().unwrap().name, "app");
        assert_eq!(child.defaults.as_ref().unwrap().replicas, Some(4));
    }

    #[test]
    fn extends_cycle_is_rejected() {
        let raw = r#"
a:
  extends: b
b:
  extends: a
"#;
        let err = resolve_extends(&deployments_map(raw)).unwrap_err().to_string();
        assert!(err.contains("Cyclic"), "unexpected error: {err}");
    }

    #[test]
    fn extends_unknown_base_is_rejected() {
        let raw = r#"
a:
  extends: nope
  primary_host: web
"#;
        let err = resolve_extends(&deployments_map(raw)).unwrap_err().to_string();
        assert!(err.contains("unknown deployment"), "unexpected error: {err}");
    }

    #[test]
    fn abstract_base_is_not_deployed_and_children_inherit() {
        let raw = r#"
type: k8s
gateway:
  hosts:
    web: example.com
  tls:
    disable: true
deployments:
  base:
    abstract: true
    application:
      name: app
    defaults:
      replicas: 3
  prod:
    extends: base
    primary_host: web
"#;
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(raw).unwrap();
        let root = tempfile::tempdir().unwrap();
        let spec = convert_env_spec(yaml, root.path(), None).unwrap();

        // The abstract template is not itself deployed.
        assert_eq!(spec.deployments.len(), 1);
        let prod = &spec.deployments[0];
        assert_eq!(prod.name, "prod");
        assert_eq!(prod.primary_host, "web");
        // Fields inherited from the abstract base.
        assert_eq!(prod.application.name, "app");
        assert_eq!(prod.defaults.replicas, 3);
    }

    /// A local env spec with two deployments that deliberately claim the same
    /// domain and path — only one of them ever runs.
    const LOCAL_SIBLINGS: &str = r#"
type: local
gateway:
  hosts:
    web: localhost:4090
  tls:
    disable: true
deployments:
  local:
    primary_host: web
    application:
      name: app
    services:
      customer:
        host: web
        prefix: /
        ports:
          - "4003:80"
  with-prod-firebase:
    extends: local
"#;

    #[test]
    fn local_keeps_only_the_selected_deployment() {
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(LOCAL_SIBLINGS).unwrap();
        let root = tempfile::tempdir().unwrap();
        let spec = convert_env_spec(yaml, root.path(), Some("with-prod-firebase")).unwrap();

        // The sibling is not part of this run, so its identical route is not a conflict.
        assert_eq!(spec.deployments.len(), 1);
        assert_eq!(spec.deployments[0].name, "with-prod-firebase");
    }

    #[test]
    fn local_without_selection_still_requires_one() {
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(LOCAL_SIBLINGS).unwrap();
        let root = tempfile::tempdir().unwrap();
        let err = convert_env_spec(yaml, root.path(), None).unwrap_err().to_string();
        assert!(err.contains("--deployment"), "unexpected error: {err}");
    }

    #[test]
    fn local_unknown_selected_deployment_is_rejected() {
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(LOCAL_SIBLINGS).unwrap();
        let root = tempfile::tempdir().unwrap();
        let err = convert_env_spec(yaml, root.path(), Some("nope")).unwrap_err().to_string();
        assert!(err.contains("not found in env spec"), "unexpected error: {err}");
        assert!(err.contains("local, with-prod-firebase"), "unexpected error: {err}");
    }

    #[test]
    fn non_local_keeps_every_deployment_when_one_is_selected() {
        // K8S deployments run side by side, so selecting one must not hide the others
        // (their cross-deployment routing conflicts are real).
        let raw = r#"
type: k8s
gateway:
  hosts:
    web: example.com
  tls:
    disable: true
deployments:
  staging:
    primary_host: web
    application:
      name: app
  prod:
    primary_host: web
    application:
      name: app
"#;
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(raw).unwrap();
        let root = tempfile::tempdir().unwrap();
        let spec = convert_env_spec(yaml, root.path(), Some("prod")).unwrap();
        assert_eq!(spec.deployments.len(), 2);
    }

    #[test]
    fn missing_required_field_after_merge_is_rejected() {
        // `prod` inherits nothing that supplies primary_host, so conversion fails.
        let raw = r#"
type: k8s
gateway:
  hosts:
    web: example.com
  tls:
    disable: true
deployments:
  base:
    abstract: true
    application:
      name: app
  prod:
    extends: base
"#;
        let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(raw).unwrap();
        let root = tempfile::tempdir().unwrap();
        let err = convert_env_spec(yaml, root.path(), None).unwrap_err().to_string();
        assert!(err.contains("primary_host"), "unexpected error: {err}");
    }
}
