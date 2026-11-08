//! `simpled controller`: the environment's control-plane service, next to the
//! gateway. It is served under `/simpled/` on the env spec's `controller.host`,
//! and answers health, versions and the private API from the labels of the
//! running services (`docs/design/controller.md`).

pub mod acme;
pub mod api;
pub mod aws;
pub mod cloudwatch;
pub mod http;
pub mod inspect;
pub mod model;
pub mod source;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// Written by `prepare-deployment` and mounted into the controller as a config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub simpled_version: String,
    pub mode: source::Mode,
    /// Each deployment of the env spec, to the domains its services are routed
    /// on: the certificates its health reports on.
    #[serde(default)]
    pub domains: std::collections::BTreeMap<String, Vec<String>>,
    /// `controller` with a token: the private API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiConfig>,
    /// Swarm with `tls.letsencrypt`: the controller issues the gateway's certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acme: Option<acme::AcmeConfig>,
    /// `controller.cloudwatch`: health metrics and their alarms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloudwatch: Option<cloudwatch::CloudWatchConfig>,
}

/// The private API's tokens, as files the controller reads at start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_token_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manage_token_file: Option<String>,
}

/// Where a token is mounted into the controller's container.
pub fn token_path(token: &str) -> String {
    format!("/run/secrets/simpled_controller_{}", token)
}

/// Where the controller looks for a token file, relative to the deploy directory.
pub fn token_file(dir: &str, token: &str) -> String {
    format!("{}/{}", dir, token)
}

/// Writes the controller's token files under `dir`, relative to `output_dir`: a
/// token read when the deployment was prepared now, an `aws` one by the fetch
/// script on the deploy target. Returns the tokens that are set.
pub fn write_token_files(
    spec: &crate::resolved_spec::EnvironmentResolvedSpec,
    output_dir: &Path,
    dir: &str,
    fetch_script: &mut crate::secret_fetch::FetchScript,
) -> Result<Vec<&'static str>> {
    let Some(controller) = &spec.ingress.controller else {
        return Ok(vec![]);
    };
    let mut tokens = Vec::new();
    for (name, token) in controller.tokens() {
        let relative = token_file(dir, name);
        match token.deferred() {
            Some(reference) => {
                fetch_script.fetch(token, reference);
                // What prepare-deployment checks on a literal token, checked here
                // on the fetched value, before the deploy: the controller would
                // otherwise refuse the token and the API would not exist. The
                // value is trimmed first, as the controller trims the file, so
                // both count the same characters.
                fetch_script.command(format!(
                    "{var}=\"${{{var}#\"${{{var}%%[![:space:]]*}}\"}}\"; {var}=\"${{{var}%\"${{{var}##*[![:space:]]}}\"}}\"",
                    var = token.shell_var(),
                ));
                fetch_script.command(format!(
                    "if [ \"${{#{var}}}\" -lt {min} ]; then echo 'controller.{name} fetched from AWS is '\"${{#{var}}}\"' \
                     characters long; a token needs at least {min}, for example `openssl rand -hex 32`' >&2; exit 1; fi",
                    var = token.shell_var(),
                    min = crate::resolver::MIN_TOKEN_LENGTH,
                    name = name,
                ));
                fetch_script.write_to_file(token, &relative);
            }
            None => {
                let path = output_dir.join(&relative);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, token.literal().unwrap_or_default())
                    .with_context(|| format!("Cannot write {}", path.display()))?;
            }
        }
        tokens.push(name);
    }
    Ok(tokens)
}

/// The time now, in Unix seconds.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// The path the gateway serves the controller under on `controller.host`, and
/// strips before forwarding.
pub const PREFIX: &str = "/simpled";

/// Where the controller listens, inside its container.
pub const PORT: u16 = 8080;
/// The network only the gateway and the controller are on: app containers cannot
/// reach the controller except through the gateway's routes.
pub const CONTROL_NETWORK: &str = "simpled_control";
/// The name the gateway reaches the controller by: its Swarm service name in the
/// `simpled` stack, and a network alias of the standalone container.
pub const HOST: &str = "simpled_controller";
/// Overrides the image, for a private registry or a local test build.
pub const IMAGE_VAR: &str = "SIMPLED_CONTROLLER_IMAGE";
/// Where the generated stack mounts `controller.json`.
pub const CONFIG_PATH: &str = "/etc/simpled/controller.json";

/// The image of this simpled version, which the gateway's stack pins.
pub fn image() -> String {
    format!("ghcr.io/vkubiv/simpled:{}", env!("CARGO_PKG_VERSION"))
}

/// `${SIMPLED_CONTROLLER_IMAGE:-<image>}`, read by `docker stack deploy` and the shell alike.
pub fn image_reference() -> String {
    format!("${{{}:-{}}}", IMAGE_VAR, image())
}

/// Each deployment of the env spec to the domains its services are routed on,
/// sorted. A deployment with no routes, workers only, is still listed: the
/// controller answers `down` for it until its services run, not 404.
pub fn deployment_domains(
    spec: &crate::resolved_spec::EnvironmentResolvedSpec,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut domains: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for rule in &spec.ingress.rules {
        for service in &rule.services {
            let list = domains.entry(service.deployment_name.clone()).or_default();
            if !list.contains(&rule.domain_name) {
                list.push(rule.domain_name.clone());
            }
        }
    }
    for deployment in &spec.ingress.deployments {
        domains.entry(deployment.clone()).or_default();
    }
    for list in domains.values_mut() {
        list.sort();
    }
    domains
}

/// The controller's config for an environment.
pub fn config_for(spec: &crate::resolved_spec::EnvironmentResolvedSpec, mode: source::Mode) -> Config {
    Config {
        simpled_version: env!("CARGO_PKG_VERSION").to_string(),
        mode,
        domains: deployment_domains(spec),
        api: spec.ingress.controller.as_ref().and_then(|d| {
            let path =
                |t: &Option<crate::resolved_spec::SecretResolvedSpec>, name: &str| t.as_ref().map(|_| token_path(name));
            let api = ApiConfig {
                read_token_file: path(&d.read_token, "read_token"),
                manage_token_file: path(&d.manage_token, "manage_token"),
            };
            (api.read_token_file.is_some() || api.manage_token_file.is_some()).then_some(api)
        }),
        acme: match mode {
            source::Mode::Swarm => spec
                .ingress
                .tls
                .as_ref()
                .and_then(|t| t.letsencrypt.as_ref())
                .map(|le| acme::AcmeConfig {
                    directory: le.server.clone(),
                    email: le.email.clone(),
                    domains: crate::gateway_tls::domains(&spec.ingress),
                    ca_file: None,
                })
                .filter(|a| !a.domains.is_empty()),
            // One host: Traefik is its own ACME client there.
            source::Mode::Standalone => None,
        },
        cloudwatch: spec.ingress.controller.as_ref().and_then(|c| {
            let cw = c.cloudwatch.as_ref()?;
            let scheme = if spec.ingress.tls.is_some() { "https" } else { "http" };
            let host = c.domains.first().cloned().unwrap_or_default();
            Some(cloudwatch::CloudWatchConfig {
                name: cw.name.clone(),
                region: cw.region.clone(),
                alarm_email: cw.alarm_email.clone(),
                health_url: format!("{}://{}{}/health", scheme, host, PREFIX),
            })
        }),
    }
}

/// Writes `controller.json` into `dir`.
pub fn write_config(dir: &Path, config: &Config) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("controller.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(config)?
            + "
",
    )
    .with_context(|| format!("Cannot write {}", path.display()))
}

pub fn run(config: &Path, listen: &str) -> Result<()> {
    let text = std::fs::read_to_string(config).with_context(|| format!("Cannot read {}", config.display()))?;
    let config: Config =
        serde_json::from_str(&text).with_context(|| format!("{} is not a controller config", config.display()))?;
    if config.simpled_version != env!("CARGO_PKG_VERSION") {
        eprintln!(
            "Warning: the config was written by simpled {}, this controller is {}",
            config.simpled_version,
            env!("CARGO_PKG_VERSION")
        );
    }

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let challenges = acme::Challenges::default();
        // One connection; bollard's `Docker` is a cheap clone of it.
        let docker = source::connect_docker()?;
        let certs = match &config.acme {
            Some(acme_config) => {
                let domains_of = config
                    .domains
                    .iter()
                    .map(|(deployment, domains)| {
                        let with_cert = domains.iter().filter(|d| acme_config.domains.contains(d)).cloned();
                        (deployment.clone(), with_cert.collect())
                    })
                    .collect();
                Some(http::Certs {
                    source: Box::new(source::CachedCerts::new(docker.clone())),
                    domains_of,
                    domains: acme_config.domains.clone(),
                })
            }
            None => None,
        };
        if let Some(acme_config) = config.acme.clone() {
            eprintln!(
                "Issuing the gateway's certificates for {} domain(s) from {}",
                acme_config.domains.len(),
                acme_config.directory
            );
            tokio::spawn(acme::run(acme_config, docker.clone(), challenges.clone()));
        }
        let api = config.api.as_ref().and_then(|a| {
            api::Api::new(
                api::Api::read_token(a.read_token_file.as_deref()),
                api::Api::read_token(a.manage_token_file.as_deref()),
            )
        });
        if api.is_some() {
            eprintln!("Serving the private API under {}/api/v1", PREFIX);
        }
        let app = Arc::new(http::App {
            api,
            certs,
            challenges,
            deployments: config.domains.keys().cloned().collect(),
            source: Box::new(source::DockerSource::new(docker, config.mode)),
        });
        if let Some(cloudwatch_config) = config.cloudwatch.clone() {
            tokio::spawn(cloudwatch::run(cloudwatch_config, app.clone()));
        }
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .with_context(|| format!("Cannot listen on {}", listen))?;
        eprintln!(
            "simpled controller {} listening on {}, served under {}/",
            env!("CARGO_PKG_VERSION"),
            listen,
            PREFIX
        );
        axum::serve(listener, http::router(app))
            .with_graceful_shutdown(shutdown())
            .await
            .context("The controller's server failed")
    })
}

/// Docker stops a container with SIGTERM; as PID 1 nothing else handles it.
async fn shutdown() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::{
        DeploymentResolvedSpec, EnvironmentResolvedSpec, IngressResolvedSpec, IngressRule, IngressToServiceRule,
    };
    use crate::spec::{DeploymentEnvType, DockerSpecificSpec};

    #[test]
    fn every_deployment_is_listed_with_the_domains_its_services_are_routed_on() {
        let to = |deployment: &str, service: &str| IngressToServiceRule {
            service_name: service.to_string(),
            deployment_name: deployment.to_string(),
            port: 80,
            prefix: "/".to_string(),
            strip_prefix: false,
            body_limit: None,
        };
        let rule = |domain: &str, services: Vec<IngressToServiceRule>| IngressRule {
            domain_name: domain.to_string(),
            services,
        };
        let spec = EnvironmentResolvedSpec {
            ports: Default::default(),
            env_type: DeploymentEnvType::Docker(DockerSpecificSpec {
                swarm_mode: true,
                multi_node: false,
                target_platform: Default::default(),
            }),
            ingress: IngressResolvedSpec {
                // Every deployment of the env spec, workers (no routes) included.
                deployments: vec!["prod".to_string(), "staging".to_string(), "workers".to_string()],
                version_routes: vec![],
                version_document: None,
                controller: None,
                trusted_proxies: Vec::new(),
                access_log: None,
                global: false,
                placement: None,
                name: "gateway".to_string(),
                tls: None,
                domains: vec![],
                rules: vec![
                    rule("www.example.com", vec![to("prod", "web"), to("prod", "api")]),
                    rule("api.example.com", vec![to("prod", "api")]),
                    rule("staging.example.com", vec![to("staging", "web")]),
                ],
                redirects: vec![],
            },
            current_deployment: DeploymentResolvedSpec {
                name: "workers".to_string(),
                application_name: "shop".to_string(),
                application_version: "1.0.0".to_string(),
                configs: vec![],
                secrets: vec![],
                services: vec![],
                volumes: vec![],
                volume_bindings: Default::default(),
                host_environment: vec![],
            },
        };
        let domains = deployment_domains(&spec);
        assert_eq!(
            domains,
            std::collections::BTreeMap::from([
                (
                    "prod".to_string(),
                    vec!["api.example.com".to_string(), "www.example.com".to_string()]
                ),
                ("staging".to_string(), vec!["staging.example.com".to_string()]),
                ("workers".to_string(), vec![]),
            ])
        );
    }
}
