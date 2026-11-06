//! `simpled status`, `simpled logs` and `simpled wait`: clients of the controller
//! (`docs/design/controller.md` §4). The first two use the private API, which the
//! env spec routes under `controller.hosts`, with the token in
//! `SIMPLED_TOKEN`. `wait` polls a deployment's public `/health`.

use crate::spec::DeploymentEnvironmentSpec;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::time::{Duration, Instant};

pub const TOKEN_VAR: &str = "SIMPLED_TOKEN";

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

/// `https://<domain><prefix>/api/v1`, from the first of `controller.hosts`.
pub fn api_base(env: &DeploymentEnvironmentSpec) -> Result<String> {
    let (alias, prefix) = env
        .ingress
        .controller
        .as_ref()
        .and_then(|d| d.hosts.first())
        .ok_or_else(|| {
            anyhow!("The env spec routes no private API: set controller.hosts and a token, or pass --url")
        })?;
    Ok(format!(
        "{}://{}{}/api/v1",
        scheme(env),
        first_domain(env, alias)?,
        prefix.trim_end_matches('/')
    ))
}

/// The public health of a deployment, on its primary host.
pub fn health_url(env: &DeploymentEnvironmentSpec, deployment: &str) -> Result<String> {
    let dep = env
        .deployments
        .iter()
        .find(|d| d.name == deployment)
        .ok_or_else(|| anyhow!("The env spec has no deployment '{}'", deployment))?;
    Ok(format!(
        "{}://{}{}",
        scheme(env),
        first_domain(env, &dep.primary_host)?,
        crate::controller::http::HEALTH_PATH
    ))
}

fn token() -> Result<String> {
    std::env::var(TOKEN_VAR)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().to_string())
        .ok_or_else(|| anyhow!("Set {} to the controller's read or manage token", TOKEN_VAR))
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
fn refused(status: reqwest::StatusCode, url: &str, what: &str) -> anyhow::Error {
    match status.as_u16() {
        401 => anyhow!("The controller refused the token in {} (401)", TOKEN_VAR),
        403 => anyhow!("The token in {} may not do that (403)", TOKEN_VAR),
        404 => anyhow!("{}: not found (404) at {}", what, url),
        429 => anyhow!("Too many failed attempts from here; try again in a minute (429)"),
        _ => anyhow!("{} answered {}", url, status),
    }
}

fn get_json(url: &str, token: &str) -> Result<Value> {
    let response = client(Some(Duration::from_secs(30)))?
        .get(url)
        .bearer_auth(token)
        .send()
        .with_context(|| format!("Cannot reach {}", url))?;
    if !response.status().is_success() {
        return Err(refused(response.status(), url, "the controller's API"));
    }
    response.json().with_context(|| format!("{} did not answer JSON", url))
}

/// `simpled status`.
pub fn status(base: &str, json: bool) -> Result<()> {
    let body = get_json(&format!("{}/status", base), &token()?)?;
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
pub fn logs(base: &str, args: &LogArgs) -> Result<()> {
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
        .bearer_auth(token()?)
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
        return Err(refused(response.status(), &url, &what));
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
    loop {
        match client.get(url).send() {
            // 503 still carries the health document.
            Ok(response) => match response.json::<Value>() {
                Ok(health) => {
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
                Err(_) => last = "an answer that is not the health document".to_string(),
            },
            Err(e) if certificate_error(&e) => {
                last = format!("its certificate is not trusted ({}); --insecure skips the check", e)
            }
            Err(e) => last = format!("cannot reach it: {}", e),
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
