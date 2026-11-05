//! The daemon's private API, under `gateway.daemon.hosts`
//! (`docs/design/daemon.md` §4). Every route needs `Authorization: Bearer
//! <token>`. The read token reads state; the manage token can do everything the
//! read token can, and the write routes later. Without a token the routes do
//! not exist.

use super::http::App;
use super::model;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Failed attempts a client may make in `FAILURE_WINDOW` before it is refused.
const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Read,
    Manage,
}

pub struct Api {
    read_token: Option<String>,
    manage_token: Option<String>,
    /// Client to (failures, start of the window).
    failures: Mutex<HashMap<String, (u32, Instant)>>,
}

impl Api {
    /// `None` without a usable token: then no API route exists.
    pub fn new(read_token: Option<String>, manage_token: Option<String>) -> Option<Self> {
        if read_token.is_none() && manage_token.is_none() {
            return None;
        }
        Some(Api {
            read_token,
            manage_token,
            failures: Mutex::new(HashMap::new()),
        })
    }

    /// Reads a token file. A missing, empty or short token is reported and left
    /// out, so the daemon still answers the public routes.
    pub fn read_token(path: Option<&str>) -> Option<String> {
        let path = path?;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let token = text.trim().to_string();
                if token.chars().count() < 32 {
                    eprintln!(
                        "api: the token in {} is shorter than 32 characters; it is not accepted",
                        path
                    );
                    None
                } else {
                    Some(token)
                }
            }
            Err(e) => {
                eprintln!("api: cannot read {}: {}", path, e);
                None
            }
        }
    }
}

/// Compares in time that depends only on the lengths, not on where they differ.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0)) as usize;
    }
    diff == 0
}

/// The client as the gateway saw it. Only the gateway reaches the daemon, and
/// it sets these from the connection, or from `trusted_proxies`.
fn client(headers: &HeaderMap) -> String {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    header("x-real-ip")
        .or_else(|| {
            header("x-forwarded-for")
                .and_then(|v| v.split(',').next())
                .map(str::trim)
        })
        .unwrap_or("unknown")
        .to_string()
}

fn error(status: StatusCode, message: &str) -> Response {
    let mut response = (
        status,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        json!({ "error": message }).to_string(),
    )
        .into_response();
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, header::HeaderValue::from_static("Bearer"));
    }
    response
}

/// Why a request is refused; the response is `error(status, message)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Denied(pub StatusCode, pub &'static str);

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        error(self.0, self.1)
    }
}

/// 401 without a valid token, 403 with one that does not reach `needed`, and
/// 429 for a client that keeps failing.
pub fn authorize(api: &Api, headers: &HeaderMap, needed: Access) -> Result<Access, Denied> {
    let client = client(headers);
    let now = Instant::now();
    {
        let mut failures = api
            .failures
            .lock()
            .map_err(|_| Denied(StatusCode::INTERNAL_SERVER_ERROR, "locked"))?;
        failures.retain(|_, (_, since)| now.duration_since(*since) < FAILURE_WINDOW);
        if failures.get(&client).is_some_and(|(count, _)| *count >= MAX_FAILURES) {
            return Err(Denied(
                StatusCode::TOO_MANY_REQUESTS,
                "too many failed attempts; try again in a minute",
            ));
        }
    }
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    let access = presented.and_then(|token| {
        if api.manage_token.as_deref().is_some_and(|t| same(t, token)) {
            Some(Access::Manage)
        } else if api.read_token.as_deref().is_some_and(|t| same(t, token)) {
            Some(Access::Read)
        } else {
            None
        }
    });
    match access {
        Some(access) if access >= needed => Ok(access),
        Some(_) => Err(Denied(StatusCode::FORBIDDEN, "this token cannot do that")),
        None => {
            if let Ok(mut failures) = api.failures.lock() {
                let entry = failures.entry(client).or_insert((0, now));
                entry.0 += 1;
            }
            Err(Denied(StatusCode::UNAUTHORIZED, "a valid bearer token is required"))
        }
    }
}

fn json_ok(body: serde_json::Value) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body.to_string(),
    )
        .into_response()
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/deployments/{deployment}/services", get(services))
}

fn guard(app: &App, headers: &HeaderMap) -> Result<(), Denied> {
    let api = app.api.as_ref().ok_or(Denied(StatusCode::NOT_FOUND, "not found"))?;
    authorize(api, headers, Access::Read).map(|_| ())
}

/// Every deployment: its version, health and services, for a dashboard or
/// `simpled status`.
async fn status(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Err(denied) = guard(&app, &headers) {
        return denied.into_response();
    }
    let deployments = match app.source.deployments().await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("api: {:#}", e);
            return error(StatusCode::SERVICE_UNAVAILABLE, "cannot read the deployments");
        }
    };
    let mut out = Vec::new();
    for deployment in deployments {
        let services = match app.source.services(&deployment).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("api: {}: {:#}", deployment, e);
                continue;
            }
        };
        let health = super::http::health_with_tls(&app, &deployment, model::health(&services)).await;
        out.push(json!({
            "name": deployment,
            "version": model::version_document(&deployment, &services),
            "health": health,
            "services": services.iter().map(|s| json!({
                "name": s.service,
                "desired": s.desired,
                "running": s.running,
            })).collect::<Vec<_>>(),
        }));
    }
    json_ok(json!({ "deployments": out }))
}

/// One deployment's services. A deployment the daemon does not know is a 404,
/// the same as a name it may not see, so names cannot be probed.
async fn services(State(app): State<Arc<App>>, headers: HeaderMap, Path(deployment): Path<String>) -> Response {
    if let Err(denied) = guard(&app, &headers) {
        return denied.into_response();
    }
    let services = match app.source.services(&deployment).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("api: {}: {:#}", deployment, e);
            return error(StatusCode::SERVICE_UNAVAILABLE, "cannot read the deployment's services");
        }
    };
    if services.is_empty() {
        return error(StatusCode::NOT_FOUND, "not found");
    }
    json_ok(json!({
        "deployment": deployment,
        "services": services.iter().map(|s| json!({
            "name": s.service,
            "image": s.image,
            "version": s.version,
            "desired": s.desired,
            "running": s.running,
            "unhealthy": s.unhealthy,
            "updating": s.updating,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: &str = "read-token-read-token-read-token-0001";
    const MANAGE: &str = "manage-token-manage-token-manage-0001";

    fn headers(token: Option<&str>, ip: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", ip.parse().unwrap());
        if let Some(t) = token {
            h.insert(header::AUTHORIZATION, format!("Bearer {}", t).parse().unwrap());
        }
        h
    }

    #[test]
    fn tokens_grant_their_access_and_nothing_else() {
        let api = Api::new(Some(READ.to_string()), Some(MANAGE.to_string())).unwrap();
        assert_eq!(
            authorize(&api, &headers(Some(READ), "1.1.1.1"), Access::Read).unwrap(),
            Access::Read
        );
        assert_eq!(
            authorize(&api, &headers(Some(MANAGE), "1.1.1.1"), Access::Read).unwrap(),
            Access::Manage
        );
        let status = |r: Result<Access, Denied>| r.unwrap_err().0;
        assert_eq!(
            status(authorize(&api, &headers(Some(READ), "1.1.1.1"), Access::Manage)),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(authorize(&api, &headers(None, "1.1.1.1"), Access::Read)),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status(authorize(&api, &headers(Some("nope"), "1.1.1.1"), Access::Read)),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn a_client_that_keeps_failing_is_refused_for_a_while() {
        let api = Api::new(Some(READ.to_string()), None).unwrap();
        for _ in 0..MAX_FAILURES {
            let _ = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read);
        }
        let refused = authorize(&api, &headers(Some(READ), "2.2.2.2"), Access::Read).unwrap_err();
        assert_eq!(refused.0, StatusCode::TOO_MANY_REQUESTS);
        // Another client is not affected.
        assert!(authorize(&api, &headers(Some(READ), "3.3.3.3"), Access::Read).is_ok());
    }

    #[test]
    fn no_token_means_no_api() {
        assert!(Api::new(None, None).is_none());
    }

    #[test]
    fn comparison_needs_the_whole_token() {
        assert!(same(READ, READ));
        assert!(!same(READ, &READ[..READ.len() - 1]));
        assert!(!same(READ, ""));
    }
}
