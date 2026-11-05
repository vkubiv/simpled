//! The daemon's routes. The gateway forwards the public paths of every host
//! here unchanged, so the Host header says which deployment is asked about.

use super::model::{self, Status};
use super::source::Source;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

pub const VERSION_PATH: &str = crate::version_endpoint::PATH;
pub const HEALTH_PATH: &str = "/.well-known/simpled/health";

/// The certificates the daemon issues, which a deployment's health reports on.
pub struct Certs {
    pub source: Box<dyn super::source::CertSource>,
    /// Deployment to the domains it owns that have a certificate.
    pub domains_of: HashMap<String, Vec<String>>,
}

pub struct App {
    /// The private API; `None` without a token.
    pub api: Option<super::api::Api>,
    pub certs: Option<Certs>,
    /// Answers to pending ACME challenges, which the ACME task fills in.
    pub challenges: super::acme::Challenges,
    /// Domain (lower case) to the deployment that answers for it, as
    /// `version_endpoint::routes` decided when the deployment was prepared.
    pub owners: HashMap<String, String>,
    pub source: Box<dyn Source>,
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route(VERSION_PATH, get(version))
        .route(HEALTH_PATH, get(health))
        .route("/.well-known/acme-challenge/{token}", get(challenge))
        .merge(super::api::routes())
        .with_state(app)
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
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

/// The deployment that answers on the request's host, without the port.
fn owner<'a>(app: &'a App, headers: &HeaderMap) -> Option<&'a String> {
    let host = headers.get(header::HOST)?.to_str().ok()?;
    let domain = host.rsplit_once(':').map_or(host, |(name, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            name
        } else {
            host
        }
    });
    app.owners.get(&domain.to_ascii_lowercase())
}

fn not_found() -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        json!({"error": "no deployment answers on this host"}),
    )
}

async fn version(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let Some(deployment) = owner(&app, &headers) else {
        return not_found();
    };
    match app.source.services(deployment).await {
        Ok(services) => match model::version_document(deployment, &services) {
            Some(document) => json_response(StatusCode::OK, document),
            None => json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "the deployment has no services running", "deployment": deployment}),
            ),
        },
        Err(e) => {
            eprintln!("version for {}: {:#}", deployment, e);
            json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                json!({"error": "cannot read the deployment's state"}),
            )
        }
    }
}

/// Adds the certificates of the deployment's domains to its health.
pub async fn health_with_tls(app: &App, deployment: &str, health: model::Health) -> model::Health {
    let Some(certs) = &app.certs else {
        return health;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let mut states = Vec::new();
    for domain in certs.domains_of.get(deployment).into_iter().flatten() {
        match certs.source.current(domain).await {
            Ok(current) => states.push(model::tls_state(current, now)),
            Err(e) => eprintln!("health for {}: certificate of {}: {:#}", deployment, domain, e),
        }
    }
    model::with_tls(health, &states)
}

/// The key authorization of a pending HTTP-01 challenge, on any host: the
/// token is the secret, and it is only known while the order is pending.
async fn challenge(State(app): State<Arc<App>>, axum::extract::Path(token): axum::extract::Path<String>) -> Response {
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

/// 200 only when healthy, so an uptime check needs no JSON parsing.
async fn health(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let Some(deployment) = owner(&app, &headers) else {
        return not_found();
    };
    let health = match app.source.services(deployment).await {
        Ok(services) => model::health(&services),
        Err(e) => {
            eprintln!("health for {}: {:#}", deployment, e);
            model::Health {
                status: Status::Down,
                converged: false,
                version: None,
                tls: None,
            }
        }
    };
    let health = health_with_tls(&app, deployment, health).await;
    let status = if health.status == Status::Healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    json_response(status, serde_json::to_value(&health).unwrap_or_default())
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

        fn deployments(&self) -> BoxFuture<'_, Result<Vec<String>>> {
            Box::pin(async move { Ok(vec!["prod".to_string()]) })
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

    async fn get(services: Vec<ServiceState>, host: &str, path: &str) -> (StatusCode, serde_json::Value) {
        let app = Arc::new(App {
            owners: HashMap::from([
                ("shop.example.com".to_string(), "prod".to_string()),
                ("blog.example.com".to_string(), "blog".to_string()),
            ]),
            source: Box::new(Fake(services)),
            challenges: Default::default(),
            certs: None,
            api: None,
        });
        let response = router(app)
            .oneshot(Request::get(path).header("host", host).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(f)
    }

    #[test]
    fn version_answers_for_the_deployment_that_owns_the_host() {
        let (status, body) = run(get(vec![api(2)], "Shop.Example.com:443", VERSION_PATH));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"application": "shop", "version": "1.2.3", "deployment": "prod", "deployed_at": 1790945826})
        );
    }

    #[test]
    fn health_is_200_only_when_healthy() {
        let (status, body) = run(get(vec![api(2)], "shop.example.com", HEALTH_PATH));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            json!({"status": "healthy", "converged": true, "version": "1.2.3"})
        );

        let (status, body) = run(get(vec![api(1)], "shop.example.com", HEALTH_PATH));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "degraded");
    }

    #[test]
    fn an_unknown_host_or_a_deployment_without_services_is_not_answered() {
        let (status, _) = run(get(vec![api(2)], "other.example.com", VERSION_PATH));
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, body) = run(get(vec![api(2)], "blog.example.com", HEALTH_PATH));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "down");
    }

    #[test]
    fn a_pending_challenge_is_answered_on_any_host_and_nothing_else() {
        let app = Arc::new(App {
            owners: HashMap::new(),
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
                owners: HashMap::from([("shop.example.com".to_string(), "prod".to_string())]),
                source: Box::new(Fake(vec![api(2)])),
                challenges: Default::default(),
                api: None,
                certs: Some(Certs {
                    source: Box::new(FakeCerts(current)),
                    domains_of: HashMap::from([("prod".to_string(), vec!["shop.example.com".to_string()])]),
                }),
            });
            run(async move {
                let response = router(app)
                    .oneshot(
                        Request::get(HEALTH_PATH)
                            .header("host", "shop.example.com")
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
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            (body["status"].as_str(), body["tls"].as_str()),
            (Some("degraded"), Some("self-signed"))
        );
        let (_, body) = ask(Some((true, 1)));
        assert_eq!(body["tls"], "expiring");
    }

    #[test]
    fn the_private_api_answers_with_a_token_and_does_not_exist_without_one() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";
        let call = |api: Option<super::super::api::Api>, path: &'static str, token: Option<&'static str>| {
            let app = Arc::new(App {
                owners: HashMap::new(),
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
}
