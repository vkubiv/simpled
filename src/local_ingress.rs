use crate::resolved_spec::*;
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeMap;
use std::net::TcpListener as StdTcpListener;
use std::process::{self, Command};
use std::thread;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::Router;
use axum_reverse_proxy::ReverseProxy;

/// A local gateway domain is written as "hostname" or "hostname:port".
fn split_domain(domain: &str) -> (&str, u16) {
    match domain.rsplit_once(':') {
        Some((host, port)) => match port.parse::<u16>() {
            Ok(port) => (host, port),
            Err(_) => (domain, 80),
        },
        None => (domain, 80),
    }
}

/// One redirect source bound to the port it is served on.
#[derive(Clone)]
struct LocalRedirect {
    hostname: String,
    location: String,
    permanent: bool,
}

/// A body limit bound to the path prefix it guards.
#[derive(Clone)]
struct LocalBodyLimit {
    prefix: String,
    max_bytes: u64,
}

/// Whether `path` falls under `prefix`, using the same prefix matching the
/// generated nginx and Traefik configurations use.
fn path_has_prefix(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    if prefix.is_empty() {
        return true;
    }
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/'),
        None => false,
    }
}

/// A proxy path is treated by `axum-reverse-proxy` as a root fallback when it is
/// empty or "/". Two root fallbacks cannot be merged into the same router, so we
/// only allow a single one per domain.
fn is_root_prefix(prefix: &str) -> bool {
    prefix.is_empty() || prefix == "/"
}

/// `413` when the request declares a body larger than the limit on the most
/// specific matching prefix, `None` otherwise.
///
/// The check reads `Content-Length`, which is what an upload sends and what
/// nginx rejects on in a real deployment. A chunked request that never declares
/// its length is passed through: the local gateway is here to reproduce the
/// limit a developer will hit in production, not to police the dev machine.
fn body_limit_exceeded(limits: &[LocalBodyLimit], req: &Request) -> Option<StatusCode> {
    let declared: u64 = req
        .headers()
        .get(header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()?;

    let path = req.uri().path();
    let limit = limits
        .iter()
        .filter(|l| path_has_prefix(path, &l.prefix))
        // Longest prefix wins, matching how the request is routed.
        .max_by_key(|l| l.prefix.trim_end_matches('/').len())?;

    // 0 is nginx's spelling of "no limit".
    if limit.max_bytes != 0 && declared > limit.max_bytes {
        Some(StatusCode::PAYLOAD_TOO_LARGE)
    } else {
        None
    }
}

/// The response for a request whose Host is a redirect source, or `None` when the
/// port's routed services should handle it instead.
///
/// The local gateway routes on path alone, so a redirect source sharing a port
/// with real services can only be told apart by the Host header.
fn redirect_response(
    redirects: &[LocalRedirect],
    host_header: Option<&str>,
    uri: &axum::http::Uri,
) -> Option<(StatusCode, String)> {
    let hostname = split_domain(host_header?).0;
    let redirect = redirects.iter().find(|r| r.hostname == hostname)?;
    let path_and_query = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let status = if redirect.permanent {
        StatusCode::MOVED_PERMANENTLY
    } else {
        StatusCode::FOUND
    };
    Some((status, format!("{}{}", redirect.location, path_and_query)))
}

/// Answers with a redirect when the request's Host matches a redirect source,
/// and otherwise lets the port's routed services handle it.
async fn apply_redirects(
    redirects: Vec<LocalRedirect>,
    limits: Vec<LocalBodyLimit>,
    req: Request,
    next: Next,
) -> axum::response::Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());

    if let Some((status, location)) = redirect_response(&redirects, host, req.uri()) {
        return (status, [(header::LOCATION, location)]).into_response();
    }

    if let Some(status) = body_limit_exceeded(&limits, &req) {
        return status.into_response();
    }

    next.run(req).await
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

    // Redirect sources never reach a service, so unlike routed rules they are
    // matched on the Host header rather than on the path. They are grouped by the
    // port they are served on and applied as a layer over that port's router,
    // which lets a redirect share a port with routed services.
    let mut redirects_by_port: BTreeMap<u16, Vec<LocalRedirect>> = BTreeMap::new();
    for redirect in &spec.redirects {
        let (hostname, port) = split_domain(&redirect.from_domain);
        redirects_by_port.entry(port).or_default().push(LocalRedirect {
            hostname: hostname.to_string(),
            location: redirect.target_url(false),
            permanent: redirect.permanent,
        });
    }
    // A port that only redirects still has to be bound and served.
    for port in redirects_by_port.keys() {
        routers.entry(*port).or_insert_with(|| (Router::new(), false));
    }

    // Body limits are enforced per path prefix, so they are collected per port
    // and applied in the same layer as the redirects.
    let mut limits_by_port: BTreeMap<u16, Vec<LocalBodyLimit>> = BTreeMap::new();

    for rule in &spec.rules {
        let (_, port) = split_domain(&rule.domain_name);

        for svc in &rule.services {
            // Only route to services of the deployment being run locally.
            if svc.deployment_name != current_deployment {
                continue;
            }

            if let Some(max_bytes) = svc.body_limit {
                limits_by_port.entry(port).or_default().push(LocalBodyLimit {
                    prefix: svc.prefix.clone(),
                    max_bytes,
                });
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
        let redirects = redirects_by_port.remove(&port).unwrap_or_default();
        let limits = limits_by_port.remove(&port).unwrap_or_default();
        let app = if redirects.is_empty() && limits.is_empty() {
            app
        } else {
            app.layer(middleware::from_fn(move |req: Request, next: Next| {
                let redirects = redirects.clone();
                let limits = limits.clone();
                async move { apply_redirects(redirects, limits, req, next).await }
            }))
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn redirects() -> Vec<LocalRedirect> {
        vec![LocalRedirect {
            hostname: "somesite.local".to_string(),
            location: "http://www.somesite.local:8080".to_string(),
            permanent: true,
        }]
    }

    fn response(host: Option<&str>, uri: &str) -> Option<(StatusCode, String)> {
        redirect_response(&redirects(), host, &uri.parse().unwrap())
    }

    #[test]
    fn a_matching_host_is_redirected_with_its_path_and_query() {
        let (status, location) = response(Some("somesite.local:8080"), "/shop?page=2").unwrap();
        assert_eq!(status, StatusCode::MOVED_PERMANENTLY);
        assert_eq!(location, "http://www.somesite.local:8080/shop?page=2");
    }

    #[test]
    fn the_host_matches_whether_or_not_the_header_carries_a_port() {
        assert!(response(Some("somesite.local"), "/").is_some());
    }

    #[test]
    fn another_host_on_the_same_port_falls_through_to_the_services() {
        assert!(response(Some("www.somesite.local:8080"), "/").is_none());
        assert!(response(None, "/").is_none());
    }

    fn request(path: &str, content_length: Option<u64>) -> Request {
        let mut builder = axum::http::Request::builder().uri(path);
        if let Some(length) = content_length {
            builder = builder.header(header::CONTENT_LENGTH, length.to_string());
        }
        builder.body(axum::body::Body::empty()).unwrap()
    }

    fn limits() -> Vec<LocalBodyLimit> {
        vec![
            LocalBodyLimit { prefix: "/".to_string(), max_bytes: 1024 },
            LocalBodyLimit { prefix: "/upload".to_string(), max_bytes: 1024 * 1024 },
        ]
    }

    #[test]
    fn an_oversized_body_is_rejected_with_413() {
        assert_eq!(
            body_limit_exceeded(&limits(), &request("/api", Some(2048))),
            Some(StatusCode::PAYLOAD_TOO_LARGE)
        );
    }

    #[test]
    fn the_most_specific_prefix_sets_the_limit() {
        // /upload allows far more than the gateway-wide default on "/".
        assert_eq!(body_limit_exceeded(&limits(), &request("/upload/file", Some(2048))), None);
        assert_eq!(
            body_limit_exceeded(&limits(), &request("/upload/file", Some(2 * 1024 * 1024))),
            Some(StatusCode::PAYLOAD_TOO_LARGE)
        );
        // A prefix only matches on a path segment boundary.
        assert_eq!(
            body_limit_exceeded(&limits(), &request("/uploader", Some(2048))),
            Some(StatusCode::PAYLOAD_TOO_LARGE)
        );
    }

    #[test]
    fn a_request_without_a_declared_length_is_passed_through() {
        assert_eq!(body_limit_exceeded(&limits(), &request("/api", None)), None);
    }

    #[test]
    fn a_zero_limit_means_no_limit() {
        let unlimited = vec![LocalBodyLimit { prefix: "/".to_string(), max_bytes: 0 }];
        assert_eq!(body_limit_exceeded(&unlimited, &request("/api", Some(u64::MAX))), None);
    }

    #[test]
    fn a_route_without_a_limit_is_not_capped_by_another_route() {
        let only_upload = vec![LocalBodyLimit { prefix: "/upload".to_string(), max_bytes: 16 }];
        assert_eq!(body_limit_exceeded(&only_upload, &request("/api", Some(4096))), None);
    }

    #[test]
    fn a_temporary_redirect_answers_302() {
        let mut rules = redirects();
        rules[0].permanent = false;
        let (status, _) = redirect_response(&rules, Some("somesite.local"), &"/".parse().unwrap()).unwrap();
        assert_eq!(status, StatusCode::FOUND);
    }
}
