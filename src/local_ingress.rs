use crate::resolved_spec::*;
use anyhow::{Result};
use std::thread;
use axum::{Router};
use axum_reverse_proxy::ReverseProxy;

pub fn run(spec: IngressResolvedSpec) -> Result<()> {
    thread::spawn(move || {
        // Create a new tokio runtime for the ingress server
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Failed to create tokio runtime for local ingress: {}", e);
                return;
            }
        };

        rt.block_on(async move {
            let mut handles = vec![];

            for domain in &spec.domains {
                // domain can be "hostname" or "hostname:port"
                let (host, port_str) = if let Some((h, p)) = domain.rsplit_once(':') {
                    (h, p)
                } else {
                    (domain.as_str(), "80")
                };

                let port = port_str.parse::<u16>().unwrap_or(80);
                
                let mut app = Router::new();
                let mut rules_found = false;

                for rule in &spec.rules {
                    // Match rules for this domain
                    if &rule.domain_name == domain {
                        rules_found = true;
                        for svc in &rule.services {
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
                    let bind_addr = format!("127.0.0.1:{}", port);
                    match tokio::net::TcpListener::bind(&bind_addr).await {
                        Ok(listener) => {
                            println!("Local ingress listening on {}", bind_addr);
                            handles.push(tokio::spawn(async move {
                                if let Err(e) = axum::serve(listener, app).await {
                                    eprintln!("Error serving ingress on {}: {}", bind_addr, e);
                                }
                            }));
                        }
                        Err(e) => {
                            eprintln!("Failed to bind local ingress on {}: {}", bind_addr, e);
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
