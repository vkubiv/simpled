use crate::resolved_spec::*;
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::net::TcpListener as StdTcpListener;
use std::process::{self, Command};
use std::thread;
use axum::Router;
use axum_reverse_proxy::ReverseProxy;

/// A proxy path is treated by `axum-reverse-proxy` as a root fallback when it is
/// empty or "/". Two root fallbacks cannot be merged into the same router, so we
/// only allow a single one per domain.
fn is_root_prefix(prefix: &str) -> bool {
    prefix.is_empty() || prefix == "/"
}

/// Brings down the local docker compose stack and aborts the process.
///
/// The local ingress and the services it fronts are useless without each other,
/// so a fatal ingress error must take the whole local deployment down with it.
/// `docker compose up` is started attached from `run_local`, but the containers
/// are owned by the docker daemon, so an abrupt `process::exit` from this
/// (background) ingress thread would orphan them. We therefore run
/// `docker compose down` in the generated `local_env` directory before exiting.
fn shutdown_stack_and_exit(message: &str) -> ! {
    eprintln!("{}", message);
    eprintln!("Bringing down the local docker compose stack...");
    let _ = Command::new("docker")
        .current_dir("local_env")
        .args(["compose", "down", "--remove-orphans"])
        .status();
    process::exit(1);
}

/// Starts the local ingress for a single, specific deployment.
///
/// Unlike k8s/docker, a local run only brings up the currently selected
/// deployment, so the ingress must route exclusively to that deployment's
/// services. The resolved ingress spec contains rules for every deployment, so
/// we filter by `current_deployment` here.
///
/// Any failure setting up the ingress aborts the whole local deployment: an
/// unreachable ingress means the local deployment is unusable, so there is no
/// point letting the services keep running. Ports are bound synchronously before
/// this function returns so that a bind failure (e.g. the port is already in
/// use) is reported to the caller *before* any docker compose services are
/// started, rather than orphaning them.
pub fn run(spec: IngressResolvedSpec, current_deployment: &str) -> Result<()> {
    let current_deployment = current_deployment.to_string();

    // A local run binds real sockets on the host, and a port can only be bound
    // once. The same listen port, however, can be referenced by several ingress
    // domains: the same "hostname:port" may be declared under multiple host
    // groups, and distinct hostnames can share a port (e.g. everything on :80).
    // We therefore group every matching service rule by its listen port and
    // build a single merged router per port, instead of binding once per domain
    // entry (which double-binds).
    //
    // The bool tracks whether a root ("/") fallback has already been claimed for
    // that port, since two root routes cannot coexist in one router.
    let mut routers: BTreeMap<u16, (Router, bool)> = BTreeMap::new();

    for rule in &spec.rules {
        // domain_name can be "hostname" or "hostname:port".
        let port = rule
            .domain_name
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok())
            .unwrap_or(80);

        for svc in &rule.services {
            // Only route to services of the deployment being run locally.
            if svc.deployment_name != current_deployment {
                continue;
            }

            let (app, root_fallback_set) =
                routers.entry(port).or_insert_with(|| (Router::new(), false));

            if is_root_prefix(&svc.prefix) {
                if *root_fallback_set {
                    return Err(anyhow!(
                        "Local ingress misconfiguration on port {}: multiple services map to the root path '/' for deployment '{}'",
                        port, current_deployment
                    ));
                }
                *root_fallback_set = true;
            }

            let mut target = format!("http://{}:{}", "localhost", svc.port);

            if !svc.strip_prefix {
                if !svc.prefix.starts_with('/') {
                    target.push('/');
                }
                target.push_str(&svc.prefix);
                if !svc.prefix.ends_with('/') {
                    target.push('/');
                }
            }

            let proxy = ReverseProxy::new(&svc.prefix, &target);

            // `merge` consumes and returns the router, so swap the stored one out
            // to fold the new proxy into it.
            *app = std::mem::take(app).merge(proxy);
        }
    }

    if routers.is_empty() {
        return Ok(());
    }

    // Bind every port synchronously and up-front. `std::net::TcpListener::bind`
    // fails immediately if the port is already in use, so this surfaces a bind
    // error to the caller before docker compose is started. The listeners are
    // handed to the async serving thread via `from_std`, which requires
    // non-blocking mode.
    let mut bound = Vec::new();
    for (port, (app, _)) in routers {
        let bind_addr = format!("0.0.0.0:{}", port);
        let listener = StdTcpListener::bind(&bind_addr)
            .with_context(|| format!("Failed to bind local ingress on {}", bind_addr))?;
        listener
            .set_nonblocking(true)
            .with_context(|| format!("Failed to set non-blocking mode on {}", bind_addr))?;
        bound.push((bind_addr, listener, app));
    }

    // All ports bound successfully; hand serving off to a background thread so
    // the caller can start the services in the foreground. From here on a
    // failure means docker compose is (about to be) running, so we tear it down
    // instead of leaving orphaned containers behind.
    thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => shutdown_stack_and_exit(&format!(
                "Failed to create tokio runtime for local ingress: {}",
                e
            )),
        };

        rt.block_on(async move {
            let mut handles = vec![];

            for (bind_addr, std_listener, app) in bound {
                let listener = match tokio::net::TcpListener::from_std(std_listener) {
                    Ok(listener) => listener,
                    Err(e) => shutdown_stack_and_exit(&format!(
                        "Failed to register local ingress listener on {}: {}",
                        bind_addr, e
                    )),
                };

                println!("Local ingress listening on {}", bind_addr);
                handles.push(tokio::spawn(async move {
                    if let Err(e) = axum::serve(listener, app).await {
                        shutdown_stack_and_exit(&format!(
                            "Error serving ingress on {}: {}",
                            bind_addr, e
                        ));
                    }
                }));
            }

            // Wait for all listeners
            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    Ok(())
}
