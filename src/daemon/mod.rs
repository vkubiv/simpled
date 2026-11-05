//! `simpled daemon`: the environment's control-plane service, next to the
//! gateway. It answers `/.well-known/simpled/version` and `/health` on every
//! gateway host from the labels of the running services
//! (`docs/design/daemon.md`).

pub mod acme;
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
    /// Swarm with `tls.letsencrypt`: the daemon issues the gateway's certificates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acme: Option<acme::AcmeConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub domain: String,
    pub deployment: String,
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
        if let Some(acme_config) = config.acme.clone() {
            let docker = source::connect_docker()?;
            eprintln!(
                "Issuing the gateway's certificates for {} domain(s) from {}",
                acme_config.domains.len(),
                acme_config.directory
            );
            tokio::spawn(acme::run(acme_config, docker, challenges.clone()));
        }
        let app = Arc::new(http::App {
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
