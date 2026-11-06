//! The controller's private API, under `/simpled/api/v1` on `controller.host`
//! (`docs/design/controller.md` §4). Every route needs `Authorization: Bearer
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
/// How long a client past `MAX_FAILURES` waits for each answer, per failure
/// beyond the limit, and at most. The client's requests wait one at a time, so
/// the wait bounds its guesses however many connections it opens. A valid
/// token still passes after the wait: the client is the address the gateway
/// saw, which behind a proxy is everyone's, and the token's holders must not be
/// locked out. The wait is what makes the failures cost anything; the length
/// of the token is what makes guessing hopeless.
const PENALTY_PER_FAILURE: Duration = Duration::from_secs(1);
const MAX_PENALTY: Duration = Duration::from_secs(10);

/// A client's failed attempts, from the first in the window.
struct Failing {
    count: u32,
    since: Instant,
    /// Held by one of its requests at a time, while that one waits.
    gate: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    Read,
    Manage,
}

pub struct Api {
    read_token: Option<String>,
    manage_token: Option<String>,
    failures: Mutex<HashMap<String, Failing>>,
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
    /// out, so the controller still answers the public routes.
    pub fn read_token(path: Option<&str>) -> Option<String> {
        let path = path?;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let token = text.trim().to_string();
                if token.chars().count() < crate::resolver::MIN_TOKEN_LENGTH {
                    eprintln!(
                        "api: the token in {} is shorter than {} characters; it is not accepted",
                        path,
                        crate::resolver::MIN_TOKEN_LENGTH
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

/// The client as the gateway saw it. Only the gateway reaches the controller, and
/// it sets `X-Real-Ip` from the connection, or from `trusted_proxies`. Without
/// it, the last `X-Forwarded-For` entry, which the gateway appended: the
/// entries before it are whatever the client sent.
fn client(headers: &HeaderMap) -> String {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    header("x-real-ip")
        .filter(|v| !v.is_empty())
        .or_else(|| {
            header("x-forwarded-for")
                .and_then(|v| v.rsplit(',').next())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        })
        .unwrap_or("unknown")
        .to_string()
}

fn error(status: StatusCode, message: &str) -> Response {
    let mut response = super::http::json_response(status, json!({ "error": message }));
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

/// How long a client past `MAX_FAILURES` waits before its request is looked
/// at, and the gate its requests wait at one at a time; `None` for a client
/// within the limit. Taken before `authorize`, by the handler, since the wait
/// is async.
pub fn penalty(api: &Api, headers: &HeaderMap) -> Option<(Duration, Arc<tokio::sync::Mutex<()>>)> {
    let now = Instant::now();
    let mut failures = api.failures.lock().ok()?;
    failures.retain(|_, f| now.duration_since(f.since) < FAILURE_WINDOW);
    let failing = failures.get(&client(headers))?;
    let beyond = failing.count.checked_sub(MAX_FAILURES).filter(|b| *b > 0)?;
    Some(((PENALTY_PER_FAILURE * beyond).min(MAX_PENALTY), failing.gate.clone()))
}

/// 401 without a valid token, 403 with one that does not reach `needed`, and
/// 429 instead of 401 for a client whose attempts keep failing. A valid token
/// always passes, after the `penalty` the handler made the client wait.
pub fn authorize(api: &Api, headers: &HeaderMap, needed: Access) -> Result<Access, Denied> {
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
            let now = Instant::now();
            let mut failures = api
                .failures
                .lock()
                .map_err(|_| Denied(StatusCode::INTERNAL_SERVER_ERROR, "locked"))?;
            failures.retain(|_, f| now.duration_since(f.since) < FAILURE_WINDOW);
            let failing = failures.entry(client(headers)).or_insert_with(|| Failing {
                count: 0,
                since: now,
                gate: Default::default(),
            });
            failing.count += 1;
            if failing.count > MAX_FAILURES {
                return Err(Denied(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many failed attempts; try again in a minute",
                ));
            }
            Err(Denied(StatusCode::UNAUTHORIZED, "a valid bearer token is required"))
        }
    }
}

fn json_ok(body: serde_json::Value) -> Response {
    super::http::json_response(StatusCode::OK, body)
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/deployments/{deployment}/services", get(services))
        .route("/api/v1/deployments/{deployment}/services/{service}/tasks", get(tasks))
        .route("/api/v1/deployments/{deployment}/services/{service}/logs", get(logs))
        .route("/api/v1/gateway", get(gateway))
        .route("/api/v1/nodes", get(nodes))
}

fn unavailable(what: &str, e: anyhow::Error) -> Response {
    eprintln!("api: {}: {:#}", what, e);
    error(StatusCode::SERVICE_UNAVAILABLE, "cannot read the state from Docker")
}

/// A service's tasks on Swarm: node, state, since, exit code and error, newest
/// first. Its containers on standalone Docker.
async fn tasks(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((deployment, service)): Path<(String, String)>,
) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
        return denied.into_response();
    }
    match app.source.tasks(&deployment, &service).await {
        Ok(Some(body)) => json_ok(body),
        Ok(None) => error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => unavailable("tasks", e),
    }
}

/// A service's log as `text/plain`, streamed: on Swarm, the service's log
/// aggregated across nodes. `?tail=200&since=10m&follow=1&timestamps=1`.
async fn logs(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Path((deployment, service)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
        return denied.into_response();
    }
    let now = i64::try_from(super::unix_now()).unwrap_or(i64::MAX);
    let query = match super::inspect::log_query(&params, now) {
        Ok(q) => q,
        Err(message) => return error(StatusCode::BAD_REQUEST, &message),
    };
    match app.source.logs(&deployment, &service, query).await {
        Ok(Some(stream)) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            axum::body::Body::from_stream(stream),
        )
            .into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "not found"),
        Err(e) => unavailable("logs", e),
    }
}

/// The gateway's tasks per node, and, where the controller issues them, the
/// certificates of its domains with their expiry.
async fn gateway(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
        return denied.into_response();
    }
    let mut body = match app.source.gateway().await {
        Ok(body) => body,
        Err(e) => return unavailable("gateway", e),
    };
    if let Some(certs) = &app.certs {
        let mut list = Vec::new();
        for domain in &certs.domains {
            let (issuer, expires_at) = match certs.source.current(domain).await {
                Ok(Some((true, not_after))) => ("acme", Some(not_after)),
                Ok(Some((false, _))) => ("self-signed", None),
                Ok(None) => ("none", None),
                Err(e) => return unavailable("certificates", e),
            };
            list.push(json!({ "domain": domain, "issuer": issuer, "expires_at": expires_at }));
        }
        body["certificates"] = json!(list);
    }
    json_ok(body)
}

/// The swarm's nodes: role, availability, state and labels. 404 on standalone
/// Docker, which has none.
async fn nodes(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
        return denied.into_response();
    }
    match app.source.nodes().await {
        Ok(Some(body)) => json_ok(body),
        Ok(None) => error(StatusCode::NOT_FOUND, "not a swarm"),
        Err(e) => unavailable("nodes", e),
    }
}

async fn guard(app: &App, headers: &HeaderMap) -> Result<(), Denied> {
    let api = app.api.as_ref().ok_or(Denied(StatusCode::NOT_FOUND, "not found"))?;
    if let Some((wait, gate)) = penalty(api, headers) {
        let _one_at_a_time = gate.lock().await;
        tokio::time::sleep(wait).await;
    }
    authorize(api, headers, Access::Read).map(|_| ())
}

/// Every deployment: its version, health and services, for a dashboard or
/// `simpled status`.
async fn status(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
        return denied.into_response();
    }
    // One listing of every service; the certificates are read at once after it,
    // so a dashboard's poll stays one round trip long.
    let all = match app.source.all_services().await {
        Ok(all) => all,
        Err(e) => {
            eprintln!("api: {:#}", e);
            return error(StatusCode::SERVICE_UNAVAILABLE, "cannot read the deployments");
        }
    };
    let read = |(deployment, services): (String, Vec<model::ServiceState>)| {
        let app = app.clone();
        async move {
            let health = super::http::read_health(&app, &deployment, Some(&services)).await;
            json!({
                "name": deployment,
                "version": model::version_document(&deployment, &services),
                "health": health,
                "services": services.iter().map(|s| json!({
                    "name": s.service,
                    "desired": s.desired,
                    "running": s.running,
                })).collect::<Vec<_>>(),
            })
        }
    };
    let out: Vec<_> = futures_util::future::join_all(all.into_iter().map(read)).await;
    json_ok(json!({ "deployments": out }))
}

/// One deployment's services. A deployment the controller does not know is a 404,
/// the same as a name it may not see, so names cannot be probed.
async fn services(State(app): State<Arc<App>>, headers: HeaderMap, Path(deployment): Path<String>) -> Response {
    if let Err(denied) = guard(&app, &headers).await {
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
            let refused = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read).unwrap_err();
            assert_eq!(refused.0, StatusCode::UNAUTHORIZED);
        }
        let refused = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read).unwrap_err();
        assert_eq!(refused.0, StatusCode::TOO_MANY_REQUESTS);
        // The token's holders behind the same address are not locked out.
        assert!(authorize(&api, &headers(Some(READ), "2.2.2.2"), Access::Read).is_ok());
        // Another client is not affected.
        assert!(authorize(&api, &headers(Some(READ), "3.3.3.3"), Access::Read).is_ok());
    }

    #[test]
    fn a_client_past_the_limit_waits_longer_for_each_answer() {
        let api = Api::new(Some(READ.to_string()), None).unwrap();
        let wait = |ip: &str| penalty(&api, &headers(Some("wrong"), ip)).map(|(wait, _)| wait);
        // Up to and including the tenth failure: no wait.
        for _ in 0..MAX_FAILURES {
            assert_eq!(wait("2.2.2.2"), None);
            let _ = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read);
        }
        assert_eq!(wait("2.2.2.2"), None);
        let _ = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read);
        // From the eleventh: a second per failure past the tenth, whatever the
        // client presents, including the valid token.
        assert_eq!(
            penalty(&api, &headers(Some(READ), "2.2.2.2")).map(|(wait, _)| wait),
            Some(PENALTY_PER_FAILURE)
        );
        for _ in 0..3 {
            let _ = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read);
        }
        assert_eq!(wait("2.2.2.2"), Some(PENALTY_PER_FAILURE * 4));
        for _ in 0..20 {
            let _ = authorize(&api, &headers(Some("wrong"), "2.2.2.2"), Access::Read);
        }
        assert_eq!(wait("2.2.2.2"), Some(MAX_PENALTY));
        // Its requests share one gate; another client is not affected.
        let (_, gate) = penalty(&api, &headers(Some("wrong"), "2.2.2.2")).unwrap();
        let (_, again) = penalty(&api, &headers(Some(READ), "2.2.2.2")).unwrap();
        assert!(Arc::ptr_eq(&gate, &again));
        assert_eq!(wait("3.3.3.3"), None);
    }

    #[test]
    fn the_client_is_the_address_the_gateway_saw() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "6.6.6.6, 4.4.4.4".parse().unwrap());
        // Without X-Real-Ip, the entry the gateway appended, not the client's own.
        assert_eq!(client(&h), "4.4.4.4");
        h.insert("x-real-ip", "5.5.5.5".parse().unwrap());
        assert_eq!(client(&h), "5.5.5.5");
        assert_eq!(client(&HeaderMap::new()), "unknown");
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
