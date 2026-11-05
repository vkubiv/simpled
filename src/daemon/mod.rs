//! `simpled daemon`: the environment's control-plane service, next to the
//! gateway. It answers `/.well-known/simpled/version` and `/health` on every
//! gateway host from the labels of the running services
//! (`docs/design/daemon.md`).

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub domain: String,
    pub deployment: String,
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
        let app = Arc::new(http::App {
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
