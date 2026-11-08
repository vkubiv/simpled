//! `simpled status`, `simpled logs` and `simpled wait`: clients of the controller
//! (`docs/design/controller.md` §4), served under `/simpled/` on the env spec's
//! `controller.host`. The first two use the private API, with the token from
//! `SIMPLED_TOKEN` or from the env spec's `controller.read_token` (see [`token`]).
//! `wait` polls a deployment's public health.

use crate::controller::http as controller_http;
use crate::resolved_spec::SecretResolvedValue;
use crate::spec::DeploymentEnvironmentSpec;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

pub const TOKEN_VAR: &str = "SIMPLED_TOKEN";

/// The private API's token, and where it came from, so that a refused token says
/// which source to fix.
pub struct Token {
    pub value: String,
    pub source: String,
}

fn scheme(env: &DeploymentEnvironmentSpec) -> &'static str {
    if env.ingress.tls.is_some() {
        "https"
    } else {
        "http"
    }
}

fn first_domain<'a>(env: &'a DeploymentEnvironmentSpec, alias: &str) -> Result<&'a str> {
    env.ingress
        .hosts
        .iter()
        .find(|h| h.name == alias)
        .and_then(|h| h.domain_names.first())
        .map(String::as_str)
        .ok_or_else(|| anyhow!("gateway.hosts has no domain for '{}'", alias))
}

/// `https://<domain>/simpled`: the controller, on the first domain of
/// `controller.host`.
fn controller_base(env: &DeploymentEnvironmentSpec) -> Result<String> {
    let controller = env
        .ingress
        .controller
        .as_ref()
        .ok_or_else(|| anyhow!("The env spec has no controller: set controller.host, or pass --url"))?;
    Ok(format!(
        "{}://{}{}",
        scheme(env),
        first_domain(env, &controller.host)?,
        crate::controller::PREFIX
    ))
}

/// The private API's base and the token for it, for `status` and `logs`, run next
/// to the env spec. With `--url` and `SIMPLED_TOKEN` the env spec is not read at
/// all; with `--url` alone it is read only for the token.
pub fn api_and_token(url: Option<&str>) -> Result<(String, Token)> {
    let root = Path::new(".");
    let load = || crate::spec_loader::load_env_spec(root, None, 0);
    match (url, token_from_env()) {
        (Some(url), Some(token)) => Ok((url.trim_end_matches('/').to_string(), token)),
        (Some(url), None) => {
            let env = load().with_context(|| {
                format!(
                    "{} is not set, and the env spec here, which would name the token, cannot be read",
                    TOKEN_VAR
                )
            })?;
            Ok((url.trim_end_matches('/').to_string(), token_from_spec(&env)?))
        }
        (None, token) => {
            let env = load()?;
            let base = api_base(&env)?;
            let token = match token {
                Some(token) => token,
                None => token_from_spec(&env)?,
            };
            Ok((base, token))
        }
    }
}

/// `https://<domain>/simpled/api/v1`.
pub fn api_base(env: &DeploymentEnvironmentSpec) -> Result<String> {
    Ok(format!("{}/api/v1", controller_base(env)?))
}

/// The public health of one deployment, on the controller's host.
pub fn health_url(env: &DeploymentEnvironmentSpec, deployment: &str) -> Result<String> {
    if !env.deployments.iter().any(|d| d.name == deployment) {
        bail!("The env spec has no deployment '{}'", deployment);
    }
    Ok(format!(
        "{}{}/{}",
        controller_base(env)?,
        crate::controller::http::HEALTH_PATH,
        deployment
    ))
}

/// `SIMPLED_TOKEN`, which wins: for CI, and for a one-off token.
fn token_from_env() -> Option<Token> {
    // Typed or pasted by hand, unlike a deployed value, so all surrounding
    // whitespace goes, and a variable of only whitespace counts as unset.
    let value = std::env::var(TOKEN_VAR).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| Token {
        value: value.to_string(),
        source: TOKEN_VAR.to_string(),
    })
}

/// The env spec's `controller.read_token` (or `manage_token` when it has none),
/// read the way `prepare-deployment` reads it. An `aws:` token, which a remote
/// deploy fetches only on its target, is read here like a local deployment's:
/// from [`crate::secret_fetch::LOCAL_FILE`] next to the env spec, else with the
/// AWS CLI.
fn token_from_spec(env: &DeploymentEnvironmentSpec) -> Result<Token> {
    let secret = env
        .ingress
        .controller
        .as_ref()
        .and_then(|c| c.read_token.as_ref().or(c.manage_token.as_ref()))
        .ok_or_else(|| {
            anyhow!(
                "Set {} to the controller's read or manage token: the env spec here declares neither controller.read_token nor controller.manage_token",
                TOKEN_VAR
            )
        })?;
    let name = &secret.secret_name;
    let unreadable = || format!("{} is not set, and controller.{} cannot be read here", TOKEN_VAR, name);
    let mut reader = crate::resolver::SecretReader::new(env, None);
    // An `aws` token is read here whatever the env spec's type, so that the error
    // for a refused one can say whether the file or the AWS CLI answered.
    let reference = match &secret.source {
        crate::spec::DeploymentSecretSource::Aws(reference) => reference.clone(),
        _ => match reader.value(secret).with_context(unreadable)? {
            SecretResolvedValue::Literal(value) => {
                return Ok(Token {
                    value,
                    source: format!("controller.{} of the env spec", name),
                })
            }
            SecretResolvedValue::Deferred(reference) => reference,
        },
    };
    let (value, origin) = reader.aws_here(&reference).with_context(unreadable)?;
    Ok(Token {
        value,
        source: format!(
            "controller.{} (AWS secret {}, read from {})",
            name, reference.secret_id, origin
        ),
    })
}

fn client(timeout: Option<Duration>) -> Result<reqwest::blocking::Client> {
    builder(timeout).build().context("Cannot set up the HTTP client")
}

fn builder(timeout: Option<Duration>) -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("simpled/", env!("CARGO_PKG_VERSION")))
        .timeout(timeout)
}

/// What a refused request means, in words.
fn refused(status: reqwest::StatusCode, url: &str, what: &str, token: &Token) -> anyhow::Error {
    match status.as_u16() {
        401 => anyhow!("The controller refused the token from {} (401)", token.source),
        403 => anyhow!("The token from {} may not do that (403)", token.source),
        404 => anyhow!("{}: not found (404) at {}", what, url),
        429 => anyhow!("Too many failed attempts from here; try again in a minute (429)"),
        _ => anyhow!("{} answered {}", url, status),
    }
}

fn get_json(url: &str, token: &Token) -> Result<Value> {
    let response = client(Some(Duration::from_secs(30)))?
        .get(url)
        .bearer_auth(&token.value)
        .send()
        .with_context(|| format!("Cannot reach {}", url))?;
    if !response.status().is_success() {
        return Err(refused(response.status(), url, "the controller's API", token));
    }
    response.json().with_context(|| format!("{} did not answer JSON", url))
}

/// `simpled status`.
pub fn status(base: &str, token: &Token, json: bool) -> Result<()> {
    let body = get_json(&format!("{}/status", base), token)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        print!("{}", render_status(&body));
    }
    Ok(())
}

/// The status as a few lines per deployment.
pub fn render_status(body: &Value) -> String {
    let mut out = String::new();
    let deployments = body["deployments"].as_array().cloned().unwrap_or_default();
    if deployments.is_empty() {
        out.push_str("No deployments are running.\n");
    }
    for d in deployments {
        let health = &d["health"];
        let mut flags = vec![health["status"].as_str().unwrap_or("unknown").to_string()];
        if health["converged"] == Value::Bool(false) {
            flags.push("updating".to_string());
        }
        if let Some(tls) = health["tls"].as_str().filter(|t| *t != "valid") {
            flags.push(format!("certificate {}", tls));
        }
        out.push_str(&format!(
            "{}  {} {}  {}\n",
            d["name"].as_str().unwrap_or("?"),
            d["version"]["application"].as_str().unwrap_or("?"),
            d["version"]["version"].as_str().unwrap_or("?"),
            flags.join(", ")
        ));
        for s in d["services"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  {:<24} {}/{}\n",
                s["name"].as_str().unwrap_or("?"),
                s["running"],
                s["desired"]
            ));
        }
    }
    out
}

pub struct LogArgs<'a> {
    pub deployment: &'a str,
    pub service: &'a str,
    pub follow: bool,
    pub tail: Option<&'a str>,
    pub since: Option<&'a str>,
    pub timestamps: bool,
}

/// `simpled logs`: copies the stream to stdout as it arrives.
pub fn logs(base: &str, token: &Token, args: &LogArgs) -> Result<()> {
    let mut query: Vec<(&str, &str)> = Vec::new();
    if let Some(tail) = args.tail {
        query.push(("tail", tail));
    }
    if let Some(since) = args.since {
        query.push(("since", since));
    }
    if args.follow {
        query.push(("follow", "1"));
    }
    if args.timestamps {
        query.push(("timestamps", "1"));
    }
    let url = format!(
        "{}/deployments/{}/services/{}/logs",
        base, args.deployment, args.service
    );
    // No timeout: a followed log runs until it is interrupted.
    let mut response = client(None)?
        .get(&url)
        .query(&query)
        .bearer_auth(&token.value)
        .send()
        .with_context(|| format!("Cannot reach {}", url))?;
    if !response.status().is_success() {
        let what = format!("Service '{}' of deployment '{}'", args.service, args.deployment);
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            let message = response
                .json::<Value>()
                .ok()
                .and_then(|v| v["error"].as_str().map(String::from))
                .unwrap_or_default();
            bail!("The controller refused the query: {}", message);
        }
        return Err(refused(response.status(), &url, &what, token));
    }
    let mut stdout = std::io::stdout().lock();
    std::io::copy(&mut response, &mut stdout).context("The log stream broke off")?;
    Ok(())
}

/// Whether a `/health` answer is the end of a rollout to `version`. The
/// services decide it: a certificate that is not issued yet, or close to
/// expiry, degrades `status` but says nothing about the rollout, and is
/// reported instead.
pub fn reached(health: &Value, version: &str) -> bool {
    let services = health
        .get("services_status")
        .filter(|s| s.is_string())
        .unwrap_or(&health["status"]);
    services == "healthy" && health["converged"] == Value::Bool(true) && health["version"] == version
}

/// Whether an error is TLS refusing the server's certificate.
fn certificate_error(e: &reqwest::Error) -> bool {
    let mut source: Option<&dyn std::error::Error> = Some(e);
    while let Some(err) = source {
        if err.to_string().to_ascii_lowercase().contains("certificate") {
            return true;
        }
        source = err.source();
    }
    false
}

fn describe(health: &Value) -> String {
    format!(
        "{} at {}{}",
        health["status"].as_str().unwrap_or("unknown"),
        health["version"].as_str().unwrap_or("no version"),
        if health["converged"] == Value::Bool(true) {
            ""
        } else {
            ", still updating"
        }
    )
}

/// Polls in a row that have to say the controller does not know the deployment
/// before the wait gives up on it: right after a deploy that added it, on a swarm
/// with several managers, the first may still reach a controller task with the
/// config from before.
const NOT_FOUND_POLLS: u32 = 3;

/// `simpled wait`: polls the deployment's public health until it reports
/// `version`, converged and healthy, or the time is up. `insecure` accepts any
/// certificate, for a staging or private ACME server, or the placeholder.
pub fn wait(url: &str, version: &str, timeout: Duration, interval: Duration, insecure: bool) -> Result<()> {
    let client = builder(Some(Duration::from_secs(15)))
        .danger_accept_invalid_certs(insecure)
        .build()
        .context("Cannot set up the HTTP client")?;
    let started = Instant::now();
    let mut last = String::from("no answer yet");
    let mut not_found = 0;
    loop {
        let mut still_not_found = false;
        match client.get(url).send() {
            // Health is in the body, 200 or 503 (a 2.1 controller answers
            // 503 when not healthy, and any controller when it cannot read Docker). The controller's own 404,
            // repeated, says the deployment is not one it knows, which waiting
            // does not change; any other 404 is the gateway before it routes the
            // controller, right after a deploy, and is waited out.
            Ok(response) => match (response.status(), response.json::<Value>()) {
                (reqwest::StatusCode::NOT_FOUND, Ok(body))
                    if body["error"].as_str() == Some(controller_http::NO_SUCH_DEPLOYMENT) =>
                {
                    still_not_found = true;
                    not_found += 1;
                    if not_found >= NOT_FOUND_POLLS {
                        bail!(
                            "{} is not a deployment the controller knows ({}); check the name against the env spec",
                            url,
                            controller_http::NO_SUCH_DEPLOYMENT
                        )
                    }
                    last = format!(
                        "not a deployment the controller knows ({})",
                        controller_http::NO_SUCH_DEPLOYMENT
                    );
                }
                (_, Ok(health)) if health.get("status").is_some() => {
                    if reached(&health, version) {
                        eprintln!("{} is healthy at {}", url, version);
                        if let Some(tls) = health["tls"].as_str().filter(|t| *t != "valid") {
                            eprintln!("Warning: its certificate is {}", tls);
                        }
                        return Ok(());
                    }
                    let now = describe(&health);
                    if now != last {
                        eprintln!("Waiting: {}", now);
                        last = now;
                    }
                }
                (status, Ok(other)) => {
                    last = format!(
                        "HTTP {}: {}",
                        status.as_u16(),
                        other["error"]
                            .as_str()
                            .unwrap_or("an answer that is not the health document")
                    )
                }
                (status, Err(_)) => {
                    last = format!(
                        "HTTP {} with an answer that is not the health document",
                        status.as_u16()
                    )
                }
            },
            Err(e) if certificate_error(&e) => {
                last = format!("its certificate is not trusted ({}); --insecure skips the check", e)
            }
            Err(e) => last = format!("cannot reach it: {}", e),
        }
        if !still_not_found {
            not_found = 0;
        }
        if started.elapsed().saturating_add(interval) > timeout {
            bail!(
                "{} did not report {} as healthy within {} seconds; last: {}",
                url,
                version,
                timeout.as_secs(),
                last
            );
        }
        std::thread::sleep(interval);
    }
}

/// `30s`, `10m`, `2h`, `1d`, or plain seconds.
pub fn parse_duration(value: &str) -> Result<Duration> {
    if let Ok(seconds) = value.parse::<u64>() {
        return Ok(Duration::from_secs(seconds));
    }
    crate::controller::inspect::parse_duration(value)
        .ok_or_else(|| anyhow!("'{}' is not a duration like 30s, 10m or 2h", value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_refused_token_names_where_it_came_from() {
        let token = Token {
            value: "x".to_string(),
            source: "controller.read_token (AWS secret dev/c, read from aws-secrets.local.json)".to_string(),
        };
        let url = "https://ops/simpled/api/v1/status";
        assert_eq!(
            refused(reqwest::StatusCode::UNAUTHORIZED, url, "the API", &token).to_string(),
            "The controller refused the token from controller.read_token (AWS secret dev/c, read from aws-secrets.local.json) (401)"
        );
        assert!(refused(reqwest::StatusCode::FORBIDDEN, url, "the API", &token)
            .to_string()
            .starts_with("The token from controller.read_token"));
    }

    #[test]
    fn status_reads_as_one_line_per_deployment_and_one_per_service() {
        let body = json!({"deployments": [{
            "name": "prod",
            "version": {"application": "shop", "version": "1.2.3"},
            "health": {"status": "degraded", "converged": false, "version": "1.2.3", "tls": "expiring"},
            "services": [{"name": "api", "running": 1, "desired": 2}],
        }]});
        assert_eq!(
            render_status(&body),
            "prod  shop 1.2.3  degraded, updating, certificate expiring\n  api                      1/2\n"
        );
        assert_eq!(
            render_status(&json!({"deployments": []})),
            "No deployments are running.\n"
        );
    }

    /// The same HTTP answer to `times` connections on a local port, then the
    /// listener closes.
    fn answer(times: usize, status_line: &'static str, body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for _ in 0..times {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://127.0.0.1:{}/health/prodd", port)
    }

    #[test]
    fn waiting_stops_once_the_controller_keeps_not_knowing_the_deployment() {
        let unknown = r#"{"error":"no such deployment"}"#;
        let url = answer(NOT_FOUND_POLLS as usize, "404 Not Found", unknown);
        let started = Instant::now();
        let err = wait(&url, "1.2.3", Duration::from_secs(30), Duration::from_secs(1), false).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "did not stop after {} polls",
            NOT_FOUND_POLLS
        );
        assert!(
            err.to_string().contains("not a deployment the controller knows"),
            "{}",
            err
        );
        assert!(err.to_string().contains("no such deployment"), "{}", err);

        // Said once, by a controller task that may predate the deployment: waited out.
        let url = answer(1, "404 Not Found", unknown);
        let err = wait(&url, "1.2.3", Duration::from_secs(1), Duration::from_secs(1), false).unwrap_err();
        assert!(err.to_string().contains("did not report 1.2.3 as healthy"), "{}", err);
        assert!(
            err.to_string().contains("last: not a deployment the controller knows"),
            "{}",
            err
        );
    }

    #[test]
    fn waiting_outlasts_the_gateways_own_not_found() {
        // Before the gateway routes the controller, right after a deploy.
        let url = answer(1, "404 Not Found", "404 page not found");
        let err = wait(&url, "1.2.3", Duration::from_secs(1), Duration::from_secs(1), false).unwrap_err();
        assert!(err.to_string().contains("did not report 1.2.3 as healthy"), "{}", err);
        assert!(err.to_string().contains("HTTP 404"), "{}", err);
    }

    #[test]
    fn waiting_ends_at_the_version_healthy_and_converged() {
        let health = |status: &str, converged: bool, version: &str| json!({"status": status, "converged": converged, "version": version});
        assert!(reached(&health("healthy", true, "1.2.3"), "1.2.3"));
        assert!(!reached(&health("healthy", true, "1.2.2"), "1.2.3"));
        assert!(!reached(&health("healthy", false, "1.2.3"), "1.2.3"));
        assert!(!reached(&health("degraded", true, "1.2.3"), "1.2.3"));
        // Degraded only by a certificate: the rollout is done.
        let tls_only = json!({"status": "degraded", "converged": true, "version": "1.2.3", "tls": "self-signed", "services_status": "healthy"});
        assert!(reached(&tls_only, "1.2.3"));
        let both = json!({"status": "degraded", "converged": true, "version": "1.2.3", "tls": "expiring", "services_status": "degraded"});
        assert!(!reached(&both, "1.2.3"));
        assert_eq!(
            describe(&health("degraded", false, "1.2.3")),
            "degraded at 1.2.3, still updating"
        );
    }

    #[test]
    fn durations_take_units_or_seconds() {
        assert_eq!(parse_duration("10m").unwrap(), Duration::from_secs(600));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_duration("soon").is_err());
    }
}
