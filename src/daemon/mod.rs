//! `simpled daemon`: the environment's control-plane service, next to the
//! gateway. It answers `/.well-known/simpled/version` and `/health` on every
//! gateway host from the labels of the running services
//! (`docs/design/daemon.md`).

pub mod acme;
pub mod api;
pub mod http;
pub mod model;
pub mod source;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// Written by `prepare-deployment` and mounted into the daemon as a config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub simpled_version: String,
    pub mode: source::Mode,
    /// Which deployment answers on each domain.
    pub owners: Vec<Owner>,
    /// `gateway.daemon` with a token: the private API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ApiConfig>,
    /// Swarm with `tls.letsencrypt`: the daemon issues the gateway's certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acme: Option<acme::AcmeConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub domain: String,
    pub deployment: String,
}

/// The private API's tokens, as files the daemon reads at start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_token_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manage_token_file: Option<String>,
}

/// Where a token is mounted into the daemon's container.
pub fn token_path(token: &str) -> String {
    format!("/run/secrets/simpled_daemon_{}", token)
}

/// Where the daemon looks for a token file, relative to the deploy directory.
pub fn token_file(dir: &str, token: &str) -> String {
    format!("{}/{}", dir, token)
}

/// Writes the daemon's token files under `dir`, relative to `output_dir`: a
/// token read when the deployment was prepared now, an `aws` one by the fetch
/// script on the deploy target. Returns the tokens that are set.
pub fn write_token_files(
    spec: &crate::resolved_spec::EnvironmentResolvedSpec,
    output_dir: &Path,
    dir: &str,
    fetch_script: &mut crate::secret_fetch::FetchScript,
) -> Result<Vec<&'static str>> {
    let Some(daemon) = &spec.ingress.daemon else {
        return Ok(vec![]);
    };
    let mut tokens = Vec::new();
    for (name, token) in daemon.tokens() {
        let relative = token_file(dir, name);
        match token.deferred() {
            Some(reference) => {
                fetch_script.fetch(token, reference);
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

/// Where the daemon listens, inside its container.
pub const PORT: u16 = 8080;
/// The network only the gateway and the daemon are on: app containers cannot
/// reach the daemon except through the gateway's routes.
pub const CONTROL_NETWORK: &str = "simpled_control";
/// The name the gateway reaches the daemon by: its Swarm service name in the
/// `ingress` stack, and a network alias of the standalone container.
pub const HOST: &str = "ingress_daemon";
/// Overrides the image, for a private registry or a local test build.
pub const IMAGE_VAR: &str = "SIMPLED_DAEMON_IMAGE";
/// Where the generated stack mounts `daemon.json`.
pub const CONFIG_PATH: &str = "/etc/simpled/daemon.json";

/// The image of this simpled version, which the gateway's stack pins.
pub fn image() -> String {
    format!("ghcr.io/vkubiv/simpled:{}", env!("CARGO_PKG_VERSION"))
}

/// `${SIMPLED_DAEMON_IMAGE:-<image>}`, read by `docker stack deploy` and the shell alike.
pub fn image_reference() -> String {
    format!("${{{}:-{}}}", IMAGE_VAR, image())
}

/// The daemon's config for an environment: which deployment answers on each domain.
pub fn config_for(spec: &crate::resolved_spec::EnvironmentResolvedSpec, mode: source::Mode) -> Config {
    Config {
        simpled_version: env!("CARGO_PKG_VERSION").to_string(),
        mode,
        owners: spec
            .ingress
            .version_routes
            .iter()
            .map(|r| Owner {
                domain: r.domain_name.clone(),
                deployment: r.deployment_name.clone(),
            })
            .collect(),
        api: spec.ingress.daemon.as_ref().and_then(|d| {
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
    }
}

/// Writes `daemon.json` into `dir`.
pub fn write_config(dir: &Path, config: &Config) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("daemon.json");
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
        serde_json::from_str(&text).with_context(|| format!("{} is not a daemon config", config.display()))?;
    if config.simpled_version != env!("CARGO_PKG_VERSION") {
        eprintln!(
            "Warning: the config was written by simpled {}, this daemon is {}",
            config.simpled_version,
            env!("CARGO_PKG_VERSION")
        );
    }

    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let challenges = acme::Challenges::default();
        let certs = match &config.acme {
            Some(acme_config) => {
                let mut domains_of: std::collections::HashMap<String, Vec<String>> = Default::default();
                for owner in &config.owners {
                    if acme_config.domains.contains(&owner.domain) {
                        domains_of
                            .entry(owner.deployment.clone())
                            .or_default()
                            .push(owner.domain.clone());
                    }
                }
                Some(http::Certs {
                    source: Box::new(source::connect_docker()?),
                    domains_of,
                })
            }
            None => None,
        };
        if let Some(acme_config) = config.acme.clone() {
            let docker = source::connect_docker()?;
            eprintln!(
                "Issuing the gateway's certificates for {} domain(s) from {}",
                acme_config.domains.len(),
                acme_config.directory
            );
            tokio::spawn(acme::run(acme_config, docker, challenges.clone()));
        }
        let api = config.api.as_ref().and_then(|a| {
            api::Api::new(
                api::Api::read_token(a.read_token_file.as_deref()),
                api::Api::read_token(a.manage_token_file.as_deref()),
            )
        });
        if api.is_some() {
            eprintln!("Serving the private API under /api/v1");
        }
        let app = Arc::new(http::App {
            api,
            certs,
            challenges,
            owners: config
                .owners
                .iter()
                .map(|o| (o.domain.to_ascii_lowercase(), o.deployment.clone()))
                .collect(),
            source: Box::new(source::DockerSource::connect(config.mode)?),
        });
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .with_context(|| format!("Cannot listen on {}", listen))?;
        eprintln!(
            "simpled daemon {} listening on {}, answering for {} domain(s)",
            env!("CARGO_PKG_VERSION"),
            listen,
            app.owners.len()
        );
        axum::serve(listener, http::router(app))
            .with_graceful_shutdown(shutdown())
            .await
            .context("The daemon's server failed")
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
