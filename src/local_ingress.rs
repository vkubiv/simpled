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
            let mut handles = vec![];

            for domain in &spec.domains {
                // domain can be "hostname" or "hostname:port"
                let (_host, port_str) = if let Some((h, p)) = domain.rsplit_once(':') {
                    (h, p)
                } else {
                    (domain.as_str(), "80")
                };

                let port = port_str.parse::<u16>().unwrap_or(80);

                let mut app = Router::new();
                let mut rules_found = false;
                let mut root_fallback_set = false;

                for rule in &spec.rules {
                    // Match rules for this domain
                    if &rule.domain_name == domain {
                        for svc in &rule.services {
                            // Only route to services of the deployment being run locally.
                            if svc.deployment_name != current_deployment {
                                continue;
                            }

                            if is_root_prefix(&svc.prefix) {
                                if root_fallback_set {
                                    eprintln!(
                                        "Local ingress misconfiguration on {}: multiple services map to the root path '/' for deployment '{}'",
                                        domain, current_deployment
                                    );
                                    process::exit(1);
                                }
                                root_fallback_set = true;
                            }

                            rules_found = true;

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

                            let path = svc.prefix.clone();

                            let proxy = ReverseProxy::new(&path, &target);

                            app = app.merge(proxy);
                        }
                    }
                }

                if rules_found {
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
            }

            // Wait for all listeners
            for handle in handles {
                let _ = handle.await;
            }
        });
    });

    Ok(())
}
