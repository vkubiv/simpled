use crate::resolved_spec::*;
use anyhow::{Result};
use std::thread;
use std::process;
use axum::{Router};
use axum_reverse_proxy::ReverseProxy;

/// A proxy path is treated by `axum-reverse-proxy` as a root fallback when it is
/// empty or "/". Two root fallbacks cannot be merged into the same router, so we
/// only allow a single one per domain.
fn is_root_prefix(prefix: &str) -> bool {
    prefix.is_empty() || prefix == "/"
}

/// Starts the local ingress for a single, specific deployment.
///
/// Unlike k8s/docker, a local run only brings up the currently selected
/// deployment, so the ingress must route exclusively to that deployment's
/// services. The resolved ingress spec contains rules for every deployment, so
/// we filter by `current_deployment` here.
///
/// Any failure setting up the ingress aborts the whole process: an unreachable
/// ingress means the local deployment is unusable, so there is no point letting
/// the services keep running.
pub fn run(spec: IngressResolvedSpec, current_deployment: &str) -> Result<()> {
    let current_deployment = current_deployment.to_string();

    thread::spawn(move || {
        // Create a new tokio runtime for the ingress server
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Failed to create tokio runtime for local ingress: {}", e);
                process::exit(1);
            }
        };

        rt.block_on(async move {
            use std::collections::BTreeMap;

            // A local run binds real sockets on the host, and a port can only be
            // bound once. The same listen port, however, can be referenced by
            // several ingress domains: the same "hostname:port" may be declared
            // under multiple host groups, and distinct hostnames can share a port
            // (e.g. everything on :80). We therefore group every matching service
            // rule by its listen port and build a single merged router per port,
            // instead of binding once per domain entry (which double-binds).
            //
            // The value tracks whether a root ("/") fallback has already been
            // claimed for that port, since two root routes cannot coexist in one
            // router.
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

                    let entry = routers.entry(port).or_insert_with(|| (Router::new(), false));
                    let (app, root_fallback_set) = entry;

                    if is_root_prefix(&svc.prefix) {
                        if *root_fallback_set {
                            eprintln!(
                                "Local ingress misconfiguration on port {}: multiple services map to the root path '/' for deployment '{}'",
                                port, current_deployment
                            );
                            process::exit(1);
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

                    // `merge` consumes and returns the router, so swap the stored
                    // one out to fold the new proxy into it.
                    *app = std::mem::take(app).merge(proxy);
                }
            }

            let mut handles = vec![];

            for (port, (app, _)) in routers {
                let bind_addr = format!("0.0.0.0:{}", port);
                match tokio::net::TcpListener::bind(&bind_addr).await {
                    Ok(listener) => {
                        println!("Local ingress listening on {}", bind_addr);
                        handles.push(tokio::spawn(async move {
                            if let Err(e) = axum::serve(listener, app).await {
                                eprintln!("Error serving ingress on {}: {}", bind_addr, e);
                                process::exit(1);
                            }
                        }));
                    }
                    Err(e) => {
                        eprintln!("Failed to bind local ingress on {}: {}", bind_addr, e);
                        process::exit(1);
                    }
                }
            }

            // Wait for all listeners
            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    Ok(())
}
