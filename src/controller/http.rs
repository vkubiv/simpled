//! The controller's routes. The gateway serves them under
//! `crate::controller::PREFIX` on `controller.host` and strips the prefix, so
//! the controller sees `/health`, `/api/v1/...`. ACME challenges reach it
//! unchanged, on every host.

use super::model::{self, Status};
use super::source::Source;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The whole environment's health, public.
pub const HEALTH_PATH: &str = "/health";

/// The certificates the controller issues, which health reports on.
pub struct Certs {
    pub source: Box<dyn super::source::CertSource>,
    /// Deployment to the domains it is routed on that have a certificate.
    pub domains_of: HashMap<String, Vec<String>>,
    /// Every domain the gateway holds a certificate for.
    pub domains: Vec<String>,
}

pub struct App {
    /// The private API; `None` without a token.
    pub api: Option<super::api::Api>,
    pub certs: Option<Certs>,
    /// Answers to pending ACME challenges, which the ACME task fills in.
    pub challenges: super::acme::Challenges,
    /// The deployments of the env spec, as the controller's config names them:
    /// one of them with no services yet is `down`, any other name is a 404.
    pub deployments: Vec<String>,
    pub source: Box<dyn Source>,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(HEALTH_PATH, get(stack_health))
        .route("/health/{deployment}", get(deployment_health))
        .route("/version/{deployment}", get(version))
        .route("/.well-known/acme-challenge/{token}", get(challenge))
        .merge(super::api::routes())
        .with_state(app)
}

pub(crate) fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

/// The `error` of the 404 for a deployment the controller does not know, which
/// `simpled wait` tells from the gateway's own 404.
pub const NO_SUCH_DEPLOYMENT: &str = "no such deployment";

fn no_such_deployment() -> Response {
    json_response(StatusCode::NOT_FOUND, json!({"error": NO_SUCH_DEPLOYMENT}))
}

/// The HTTP status is about the controller, not about what it reports: 200
/// whenever it could read Docker, whatever the `status` in the body, and 503
/// only when it could not, still with the document it could build.
fn health_response<T: serde::Serialize>(read: bool, health: &T) -> Response {
    let code = if read {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    json_response(code, serde_json::to_value(health).unwrap_or_default())
}

/// A deployment's services. `Ok(None)` for a name the controller neither runs
/// nor knows from the env spec; `Err` when Docker cannot be read.
async fn known_services(app: &App, deployment: &str) -> Result<Option<Vec<model::ServiceState>>, ()> {
    match app.source.services(deployment).await {
        Ok(services) if services.is_empty() && !app.deployments.iter().any(|d| d == deployment) => Ok(None),
        Ok(services) => Ok(Some(services)),
        Err(e) => {
            eprintln!("{}: {:#}", deployment, e);
            Err(())
        }
    }
}

async fn version(State(app): State<Arc<App>>, Path(deployment): Path<String>) -> Response {
    let services = match known_services(&app, &deployment).await {
        Ok(Some(services)) => services,
        Ok(None) => return no_such_deployment(),
        Err(()) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "cannot read the deployment's state"}),
            )
        }
    };
    match model::version_document(&deployment, &services) {
        Some(document) => json_response(StatusCode::OK, document),
        None => json_response(
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "the deployment has no services running", "deployment": deployment}),
        ),
    }
}

/// Adds the certificates of the deployment's domains to its health.
pub async fn health_with_tls(app: &App, deployment: &str, health: model::Health) -> model::Health {
    let Some(certs) = &app.certs else {
        return health;
    };
    let now = super::unix_now();
    let mut states = Vec::new();
    for domain in certs.domains_of.get(deployment).into_iter().flatten() {
        match certs.source.current(domain).await {
            Ok(current) => states.push(model::tls_state(current, now)),
            Err(e) => eprintln!("health for {}: certificate of {}: {:#}", deployment, domain, e),
        }
    }
    model::with_tls(health, &states)
}

/// One deployment's health, certificates included. Services that cannot be
/// read (`None`) make it `down`, like no services at all.
pub(super) async fn read_health(
    app: &App,
    deployment: &str,
    services: Option<&[model::ServiceState]>,
) -> model::Health {
    let health = model::health(services.unwrap_or_default());
    health_with_tls(app, deployment, health).await
}

async fn deployment_health(State(app): State<Arc<App>>, Path(deployment): Path<String>) -> Response {
    let services = match known_services(&app, &deployment).await {
        Ok(Some(services)) => Some(services),
        Ok(None) => return no_such_deployment(),
        Err(()) => None,
    };
    let health = read_health(&app, &deployment, services.as_deref()).await;
    health_response(services.is_some(), &health)
}

/// The environment as `/health` reports it.
pub(super) struct Environment {
    pub health: model::StackHealth,
    /// The gateway service could be read too.
    pub complete: bool,
    /// Each certificate the controller issues: whether it was issued, and its
    /// expiry. `None` for a domain with none, or one that could not be read.
    pub certificates: Vec<Option<(bool, u64)>>,
}

/// The gateway, every certificate and every deployment that has services: one
/// listing of the services, and the rest read at once, so one slow Docker call
/// does not add to the next. `Err` when the services cannot be listed.
pub(super) async fn environment(app: &Arc<App>) -> anyhow::Result<Environment> {
    let all = app.source.all_services().await?;
    let gateway = async {
        match app.source.gateway_replicas().await {
            Ok((desired, running)) => (model::gateway_health(desired, running), true),
            Err(e) => {
                eprintln!("health: gateway: {:#}", e);
                (model::gateway_health(1, 0), false)
            }
        }
    };
    let certificates = async {
        let Some(certs) = &app.certs else {
            return vec![];
        };
        let mut states = Vec::new();
        for domain in &certs.domains {
            match certs.source.current(domain).await {
                Ok(current) => states.push(current),
                Err(e) => eprintln!("health: certificate of {}: {:#}", domain, e),
            }
        }
        states
    };
    let each = futures_util::future::join_all(all.into_iter().map(|(deployment, services)| {
        let app = app.clone();
        async move {
            let health = read_health(&app, &deployment, Some(&services)).await;
            (deployment, health)
        }
    }));
    let ((gateway, complete), certificates, each) = tokio::join!(gateway, certificates, each);
    let now = super::unix_now();
    let states: Vec<model::Tls> = certificates.iter().map(|c| model::tls_state(*c, now)).collect();
    let health = model::stack_health(gateway, &states, each.into_iter().collect::<BTreeMap<_, _>>());
    Ok(Environment {
        health,
        complete,
        certificates,
    })
}

async fn stack_health(State(app): State<Arc<App>>) -> Response {
    match environment(&app).await {
        Ok(env) => health_response(env.complete, &env.health),
        Err(e) => {
            eprintln!("health: {:#}", e);
            json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"status": "down", "error": "cannot read the deployments"}),
            )
        }
    }
}

/// What the CloudWatch task publishes: `/health`, plus every deployment of the
/// env spec, `down` while it has no services. `Err` when Docker cannot be read,
/// the gateway service included: nothing is published then.
pub(super) async fn sample(app: &Arc<App>) -> anyhow::Result<super::cloudwatch::Sample> {
    let env = environment(app).await?;
    if !env.complete {
        anyhow::bail!("cannot read the gateway service");
    }
    let mut deployments: BTreeMap<String, Status> = env
        .health
        .deployments
        .iter()
        .map(|(d, h)| (d.clone(), h.status))
        .collect();
    for deployment in &app.deployments {
        deployments.entry(deployment.clone()).or_insert(Status::Down);
    }
    let worst = deployments.values().copied().fold(env.health.status, Status::max);
    let now = super::unix_now();
    let cert_days_left = app.certs.as_ref().map(|_| {
        env.certificates
            .iter()
            .map(|c| match c {
                Some((true, not_after)) => (*not_after as f64 - now as f64).max(0.0) / 86400.0,
                _ => 0.0,
            })
            .fold(f64::INFINITY, f64::min)
    });
    Ok(super::cloudwatch::Sample {
        env: worst,
        gateway: env.health.gateway.status,
        deployments,
        // No certificate listed yet: nothing to report rather than infinity.
        cert_days_left: cert_days_left
            .filter(|d| d.is_finite())
            .map(|d| (d * 10.0).round() / 10.0),
    })
}

/// The key authorization of a pending HTTP-01 challenge, on any host: the
/// token is the secret, and it is only known while the order is pending.
async fn challenge(State(app): State<Arc<App>>, Path(token): Path<String>) -> Response {
    let answer = app.challenges.read().ok().and_then(|m| m.get(&token).cloned());
    match answer {
        Some(key_authorization) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            key_authorization,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::ServiceState;
    use super::super::source::BoxFuture;
    use super::*;
    use anyhow::Result;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    struct Fake(Vec<ServiceState>);

    impl Source for Fake {
        fn services<'a>(&'a self, deployment: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>> {
            let services = if deployment == "prod" { self.0.clone() } else { vec![] };
            Box::pin(async move { Ok(services) })
        }

        fn all_services(&self) -> BoxFuture<'_, Result<BTreeMap<String, Vec<ServiceState>>>> {
            let all = BTreeMap::from([("prod".to_string(), self.0.clone())]);
            Box::pin(async move { Ok(all) })
        }
    }

    fn api(running: u64) -> ServiceState {
        ServiceState {
            service: "api".to_string(),
            application: "shop".to_string(),
            version: "1.2.3".to_string(),
            deployed_at: 1790945826,
            desired: 2,
            running,
            unhealthy: 0,
            updating: false,
            image: String::new(),
        }
    }

    fn app_with(services: Vec<ServiceState>) -> App {
        App {
            deployments: vec!["prod".to_string(), "blog".to_string()],
            source: Box::new(Fake(services)),
            challenges: Default::default(),
            certs: None,
            api: None,
        }
    }

    async fn ask(app: App, path: &str) -> (StatusCode, serde_json::Value) {
        let response = router(Arc::new(app))
            .oneshot(
                Request::get(path)
                    .header("host", "ops.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    async fn get(services: Vec<ServiceState>, path: &str) -> (StatusCode, serde_json::Value) {
        ask(app_with(services), path).await
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn version_answers_for_the_named_deployment() {
        let (status, body) = run(get(vec![api(2)], "/version/prod"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"application": "shop", "version": "1.2.3", "deployment": "prod", "deployed_at": 1790945826})
        );
    }

    #[test]
    fn a_deployments_health_is_200_whatever_it_reports() {
        let (status, body) = run(get(vec![api(2)], "/health/prod"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"status": "healthy", "converged": true, "version": "1.2.3"})
        );

        let (status, body) = run(get(vec![api(1)], "/health/prod"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "degraded");

        let (status, body) = run(get(vec![api(0)], "/health/prod"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "down");
    }

    #[test]
    fn an_unknown_deployment_is_a_404_and_a_known_one_without_services_is_down() {
        assert_eq!(run(get(vec![api(2)], "/version/other")).0, StatusCode::NOT_FOUND);
        assert_eq!(run(get(vec![api(2)], "/health/other")).0, StatusCode::NOT_FOUND);
        let (status, body) = run(get(vec![api(2)], "/health/blog"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "down");
    }

    #[test]
    fn the_stack_health_covers_the_gateway_and_every_deployment() {
        let (status, body) = run(get(vec![api(2)], "/health"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({
                "status": "healthy",
                "gateway": {"status": "healthy", "desired": 1, "running": 1},
                "deployments": {"prod": {"status": "healthy", "converged": true, "version": "1.2.3"}},
            })
        );

        let (status, body) = run(get(vec![api(0)], "/health"));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "down");
    }

    /// Docker that cannot be read: the one trouble a 503 is for.
    struct Unreadable {
        services: bool,
    }

    impl Source for Unreadable {
        fn services<'a>(&'a self, _deployment: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>> {
            let services = self.services;
            Box::pin(async move {
                if services {
                    Ok(vec![api(2)])
                } else {
                    Err(anyhow::anyhow!("no Docker"))
                }
            })
        }

        fn all_services(&self) -> BoxFuture<'_, Result<BTreeMap<String, Vec<ServiceState>>>> {
            let services = self.services;
            Box::pin(async move {
                if services {
                    Ok(BTreeMap::from([("prod".to_string(), vec![api(2)])]))
                } else {
                    Err(anyhow::anyhow!("no Docker"))
                }
            })
        }

        fn gateway_replicas(&self) -> BoxFuture<'_, Result<(u64, u64)>> {
            Box::pin(async { Err(anyhow::anyhow!("no gateway service")) })
        }
    }

    #[test]
    fn health_is_503_only_when_the_controller_cannot_read_docker() {
        let unreadable = |services: bool| App {
            source: Box::new(Unreadable { services }),
            ..app_with(vec![])
        };
        let (status, body) = run(ask(unreadable(false), "/health/prod"));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "down");
        let (status, body) = run(ask(unreadable(false), "/health"));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "cannot read the deployments");
        // The services read, the gateway not: still a report the controller could not complete.
        let (status, body) = run(ask(unreadable(true), "/health"));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["gateway"]["status"], "down");
        assert_eq!(body["deployments"]["prod"]["status"], "healthy");
    }

    #[test]
    fn a_pending_challenge_is_answered_on_any_host_and_nothing_else() {
        let app = Arc::new(App {
            deployments: vec![],
            source: Box::new(Fake(vec![])),
            challenges: Default::default(),
            certs: None,
            api: None,
        });
        app.challenges
            .write()
            .unwrap()
            .insert("tok".to_string(), "tok.thumb".to_string());
        let ask = |path: &'static str| {
            let app = app.clone();
            run(async move {
                let response = router(app)
                    .oneshot(
                        Request::get(path)
                            .header("host", "any.example.com")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
                (status, String::from_utf8(body.to_vec()).unwrap())
            })
        };
        assert_eq!(
            ask("/.well-known/acme-challenge/tok"),
            (StatusCode::OK, "tok.thumb".to_string())
        );
        assert_eq!(ask("/.well-known/acme-challenge/other").0, StatusCode::NOT_FOUND);
    }

    struct FakeCerts(Option<(bool, u64)>);

    impl super::super::source::CertSource for FakeCerts {
        fn current<'a>(&'a self, _domain: &'a str) -> BoxFuture<'a, Result<Option<(bool, u64)>>> {
            let current = self.0;
            Box::pin(async move { Ok(current) })
        }
    }

    #[test]
    fn health_reports_the_certificates_of_the_deployments_domains() {
        let ask = |current: Option<(bool, u64)>| {
            let app = Arc::new(App {
                deployments: vec!["prod".to_string()],
                source: Box::new(Fake(vec![api(2)])),
                challenges: Default::default(),
                api: None,
                certs: Some(Certs {
                    source: Box::new(FakeCerts(current)),
                    domains_of: HashMap::from([("prod".to_string(), vec!["shop.example.com".to_string()])]),
                    domains: vec!["shop.example.com".to_string()],
                }),
            });
            run(async move {
                let response = router(app)
                    .oneshot(
                        Request::get("/health/prod")
                            .header("host", "ops.example.com")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
                (status, serde_json::from_slice::<serde_json::Value>(&body).unwrap())
            })
        };
        let far = u64::MAX / 2;
        let (status, body) = ask(Some((true, far)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["tls"], "valid");
        let (status, body) = ask(Some((false, 0)));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (
                body["status"].as_str(),
                body["tls"].as_str(),
                body["services_status"].as_str()
            ),
            (Some("degraded"), Some("self-signed"), Some("healthy"))
        );
        let (_, body) = ask(Some((true, super::super::unix_now() + 24 * 3600)));
        assert_eq!(body["tls"], "expiring");
        let (_, body) = ask(Some((true, 1)));
        assert_eq!(body["tls"], "expired");
    }

    #[test]
    fn the_private_api_answers_with_a_token_and_does_not_exist_without_one() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";
        let call = |api: Option<super::super::api::Api>, path: &'static str, token: Option<&'static str>| {
            let app = Arc::new(App {
                deployments: vec![],
                source: Box::new(Fake(vec![api_service()])),
                challenges: Default::default(),
                certs: None,
                api,
            });
            run(async move {
                let mut request = Request::get(path).header("host", "ops.example.com");
                if let Some(t) = token {
                    request = request.header("authorization", format!("Bearer {}", t));
                }
                let response = router(app).oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
                let status = response.status();
                let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
                (
                    status,
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default(),
                )
            })
        };
        let api = || super::super::api::Api::new(Some(TOKEN.to_string()), None);

        let (status, body) = call(api(), "/api/v1/status", Some(TOKEN));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["deployments"][0]["name"], "prod");
        assert_eq!(body["deployments"][0]["health"]["status"], "healthy");
        assert_eq!(
            body["deployments"][0]["services"][0],
            json!({"name": "api", "desired": 2, "running": 2})
        );

        let (status, body) = call(api(), "/api/v1/deployments/prod/services", Some(TOKEN));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["services"][0]["version"], "1.2.3");
        assert_eq!(
            call(api(), "/api/v1/deployments/other/services", Some(TOKEN)).0,
            StatusCode::NOT_FOUND
        );

        assert_eq!(call(api(), "/api/v1/status", None).0, StatusCode::UNAUTHORIZED);
        assert_eq!(call(None, "/api/v1/status", Some(TOKEN)).0, StatusCode::NOT_FOUND);
    }

    fn api_service() -> ServiceState {
        api(2)
    }

    /// A source with one service, `prod`/`api`, that has a log.
    struct WithLogs;

    impl Source for WithLogs {
        fn services<'a>(&'a self, _d: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn all_services(&self) -> BoxFuture<'_, Result<BTreeMap<String, Vec<ServiceState>>>> {
            Box::pin(async { Ok(BTreeMap::new()) })
        }
        fn logs<'a>(
            &'a self,
            deployment: &'a str,
            service: &'a str,
            query: super::super::inspect::LogQuery,
        ) -> BoxFuture<'a, Result<Option<super::super::source::LogStream>>> {
            let known = deployment == "prod" && service == "api";
            Box::pin(async move {
                if !known {
                    return Ok(None);
                }
                let line = format!("tail={}\n", query.tail);
                let stream: super::super::source::LogStream = Box::pin(futures_util::stream::iter(vec![
                    Ok(axum::body::Bytes::from("started\n")),
                    Ok(axum::body::Bytes::from(line)),
                ]));
                Ok(Some(stream))
            })
        }
        fn gateway_logs(
            &self,
            query: super::super::inspect::LogQuery,
        ) -> BoxFuture<'_, Result<Option<super::super::source::LogStream>>> {
            Box::pin(async move {
                let line = format!("gateway tail={} follow={}\n", query.tail, query.follow);
                let stream: super::super::source::LogStream =
                    Box::pin(futures_util::stream::iter(vec![Ok(axum::body::Bytes::from(line))]));
                Ok(Some(stream))
            })
        }
    }

    #[test]
    fn logs_stream_as_text_and_unknown_names_are_not_found() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";
        let call = |path: &'static str| {
            let app = Arc::new(App {
                deployments: vec![],
                source: Box::new(WithLogs),
                challenges: Default::default(),
                certs: None,
                api: super::super::api::Api::new(Some(TOKEN.to_string()), None),
            });
            run(async move {
                let request = Request::get(path)
                    .header("authorization", format!("Bearer {}", TOKEN))
                    .body(Body::empty())
                    .unwrap();
                let response = router(app).oneshot(request).await.unwrap();
                let status = response.status();
                let kind = response
                    .headers()
                    .get("content-type")
                    .map(|v| v.to_str().unwrap().to_string())
                    .unwrap_or_default();
                let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
                (status, kind, String::from_utf8(body.to_vec()).unwrap())
            })
        };
        let (status, kind, body) = call("/api/v1/deployments/prod/services/api/logs?tail=5");
        assert_eq!(status, StatusCode::OK);
        assert!(kind.starts_with("text/plain"), "{}", kind);
        assert_eq!(body, "started\ntail=5\n");
        assert_eq!(
            call("/api/v1/deployments/prod/services/other/logs").0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call("/api/v1/deployments/prod/services/api/logs?tail=lots").0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            call("/api/v1/deployments/prod/services/api/tasks").0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(call("/api/v1/nodes").0, StatusCode::NOT_FOUND);

        let (status, kind, body) = call("/api/v1/gateway/logs?tail=7&follow=1");
        assert_eq!(status, StatusCode::OK);
        assert!(kind.starts_with("text/plain"), "{}", kind);
        assert_eq!(body, "gateway tail=7 follow=true\n");
        assert_eq!(call("/api/v1/gateway/logs?since=soon").0, StatusCode::BAD_REQUEST);
    }
}
