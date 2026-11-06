//! The controller as the gateway's only ACME client, on Docker Swarm
//! (`docs/design/controller.md` §7.1). For each domain it issues a certificate when
//! there is none, only the self-signed placeholder, or one that expires within
//! 30 days. Each certificate goes into a Swarm secret, and one rolling update of
//! the gateway moves it onto every secret issued in that round.
//!
//! The HTTP-01 challenge is answered by the controller itself: every gateway task, on
//! every node, routes `/.well-known/acme-challenge/` to it, so it does not matter
//! which node Let's Encrypt reaches.

use crate::gateway_tls::{self, labels, ISSUER_ACME};
pub use crate::gateway_tls::{current, CertSecret};
use anyhow::{anyhow, bail, Context, Result};
use bollard::models::{SecretSpec, ServiceSpec};
use bollard::query_parameters::{ListSecretsOptions, UpdateServiceOptions};
use bollard::Docker;
use instant_acme::{
    Account, AuthorizationStatus, ChallengeType, Identifier, NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Part of `controller.json`, written from the env spec's `tls.letsencrypt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcmeConfig {
    /// The ACME directory URL: Let's Encrypt's, or `letsencrypt.server`.
    pub directory: String,
    pub email: String,
    pub domains: Vec<String>,
    /// A CA to trust for the ACME server, for a test or private ACME server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<String>,
}

/// Challenge token to key authorization, served under
/// `/.well-known/acme-challenge/<token>` while an order is pending.
pub type Challenges = Arc<RwLock<HashMap<String, String>>>;

pub const GATEWAY_SERVICE: &str = crate::deploy_steps::GATEWAY_SERVICE;
pub const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";
const RENEW_BEFORE_SECONDS: u64 = 30 * 24 * 3600;
/// How often the certificates are checked. Issuing waits for nothing else, so
/// this only bounds how late a renewal or a new domain is noticed.
const CHECK_EVERY: Duration = Duration::from_secs(3600);
/// Before the first check: the gateway that routes the challenges to this
/// controller is deployed in the same stack, and may not be routing yet.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
/// After a failed update of the gateway: sooner than the next check, since the
/// certificate is ready and only has to be mounted.
const ROLL_OUT_RETRY: Duration = Duration::from_secs(300);

pub fn needs_certificate(current: Option<&CertSecret>, now: u64) -> bool {
    match current {
        Some(cert) => !cert.acme || cert.not_after <= now + RENEW_BEFORE_SECONDS,
        None => true,
    }
}

/// Secrets of a domain that can go once an issued certificate exists: every
/// placeholder, and every issued one but the best two in `current`'s order (the
/// current one and the one before it, which a `docker service rollback` still
/// mounts).
pub fn stale(secrets: &[CertSecret]) -> Vec<&CertSecret> {
    if !secrets.iter().any(|s| s.acme) {
        return vec![];
    }
    let keep: Vec<&str> = gateway_tls::by_preference(secrets)
        .into_iter()
        .filter(|s| s.acme)
        .take(2)
        .map(|s| s.id.as_str())
        .collect();
    secrets.iter().filter(|s| !keep.contains(&s.id.as_str())).collect()
}

/// After the n-th failure in a row: an hour, doubling up to a day, with up to a
/// tenth more, so several controllers do not retry in step.
pub fn backoff(failures: u32, jitter: f64) -> Duration {
    let hours = 2u64.saturating_pow(failures.saturating_sub(1)).min(24);
    Duration::from_secs_f64(hours as f64 * 3600.0 * (1.0 + jitter.clamp(0.0, 1.0) / 10.0))
}

fn jitter() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    (nanos % 1000) as f64 / 1000.0
}

pub async fn domain_secrets(docker: &Docker, domain: &str) -> Result<Vec<CertSecret>> {
    let found = list_cert_secrets(docker, &format!("{}={}", labels::DOMAIN, domain)).await?;
    Ok(found.into_iter().map(|(_, secret)| secret).collect())
}

/// The certificate secrets that match a label filter, each with its domain.
pub async fn list_cert_secrets(docker: &Docker, label_filter: &str) -> Result<Vec<(String, CertSecret)>> {
    let secrets = docker
        .list_secrets(Some(ListSecretsOptions {
            filters: Some(HashMap::from([("label".to_string(), vec![label_filter.to_string()])])),
        }))
        .await
        .context("Cannot list the certificate secrets")?;
    Ok(secrets
        .into_iter()
        .filter_map(|s| {
            let spec = s.spec?;
            let labels_map = spec.labels.unwrap_or_default();
            let domain = labels_map.get(labels::DOMAIN)?.clone();
            let secret = CertSecret::from_labels(
                s.id?,
                spec.name?,
                labels_map.get(labels::ISSUER).map(String::as_str),
                labels_map.get(labels::NOT_AFTER).map(String::as_str),
                s.version.and_then(|v| v.index).unwrap_or_default(),
            );
            Some((domain, secret))
        })
        .collect())
}

/// The expiry of the first certificate in a PEM chain, in Unix seconds.
pub fn not_after(chain_pem: &str) -> Result<u64> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(chain_pem.as_bytes())
        .map_err(|e| anyhow!("The issued chain is not PEM: {}", e))?;
    let cert = pem
        .parse_x509()
        .map_err(|e| anyhow!("The issued certificate does not parse: {}", e))?;
    Ok(cert.validity().not_after.timestamp().max(0) as u64)
}

async fn account(config: &AcmeConfig) -> Result<Account> {
    let builder = match &config.ca_file {
        Some(ca) => Account::builder_with_root(ca),
        None => Account::builder(),
    }
    .context("Cannot set up the ACME client")?;
    let contact = format!("mailto:{}", config.email);
    // A new account on each start: Let's Encrypt's limits that matter are on
    // certificates, not accounts, and the controller then keeps no state.
    let (account, _) = builder
        .create(
            &NewAccount {
                contact: &[contact.as_str()],
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            config.directory.clone(),
            None,
        )
        .await
        .with_context(|| format!("Cannot create an ACME account at {}", config.directory))?;
    Ok(account)
}

/// Orders a certificate for one domain and answers its HTTP-01 challenge.
/// Returns the chain and the key, in one PEM.
async fn issue(account: &Account, domain: &str, challenges: &Challenges) -> Result<String> {
    let mut order = account
        .new_order(&NewOrder::new(&[Identifier::Dns(domain.to_string())]))
        .await
        .context("Cannot create the order")?;

    let mut tokens = Vec::new();
    let result: Result<()> = async {
        let mut authorizations = order.authorizations();
        while let Some(authz) = authorizations.next().await {
            let mut authz = authz.context("Cannot read the authorization")?;
            match authz.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                other => bail!("the authorization is {:?}", other),
            }
            let mut challenge = authz
                .challenge(ChallengeType::Http01)
                .ok_or_else(|| anyhow!("the ACME server offers no http-01 challenge"))?;
            let token = challenge.token.clone();
            challenges
                .write()
                .map_err(|_| anyhow!("challenge store poisoned"))?
                .insert(token.clone(), challenge.key_authorization().as_str().to_string());
            tokens.push(token);
            challenge.set_ready().await.context("Cannot start the challenge")?;
        }
        Ok(())
    }
    .await;
    let status = match result {
        Ok(()) => order.poll_ready(&RetryPolicy::default()).await,
        Err(e) => {
            remove_tokens(challenges, &tokens);
            return Err(e);
        }
    };
    remove_tokens(challenges, &tokens);
    let status = status.context("The order did not become ready")?;
    if status != OrderStatus::Ready {
        let detail = order
            .state()
            .error
            .as_ref()
            .map(|e| format!(": {}", e.detail.clone().unwrap_or_default()))
            .unwrap_or_default();
        bail!("the order is {:?}{}", status, detail);
    }
    let key = order.finalize().await.context("Cannot finalize the order")?;
    let chain = order
        .poll_certificate(&RetryPolicy::default())
        .await
        .context("Cannot download the certificate")?;
    Ok(format!("{}{}", chain, key))
}

fn remove_tokens(challenges: &Challenges, tokens: &[String]) {
    if let Ok(mut map) = challenges.write() {
        for token in tokens {
            map.remove(token);
        }
    }
}

async fn create_secret(docker: &Docker, domain: &str, pem: &str) -> Result<(String, String)> {
    use base64::Engine;
    let expires = not_after(pem)?;
    let name = crate::deploy_steps::object_name(&gateway_tls::secret_base(domain), pem.as_bytes());
    let id = docker
        .create_secret(SecretSpec {
            name: Some(name.clone()),
            labels: Some(
                gateway_tls::secret_labels(domain, ISSUER_ACME, expires)
                    .into_iter()
                    .collect(),
            ),
            data: Some(base64::engine::general_purpose::STANDARD.encode(pem)),
            ..Default::default()
        })
        .await
        .with_context(|| format!("Cannot create the secret {}", name))?
        .id;
    Ok((id, name))
}

/// What `repoint` found: mounts of the given domains' PEMs, and how many of
/// them it pointed at another secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repointed {
    pub mounted: usize,
    pub changed: usize,
}

/// Points the gateway's mount of each domain's PEM at that domain's current
/// secret. One update, which Swarm rolls out with the gateway's own update
/// policy.
pub fn repoint(spec: &mut ServiceSpec, current: &[(String, String, String)]) -> Repointed {
    let mut result = Repointed { mounted: 0, changed: 0 };
    let secrets = spec
        .task_template
        .as_mut()
        .and_then(|t| t.container_spec.as_mut())
        .and_then(|c| c.secrets.as_mut());
    let Some(secrets) = secrets else {
        return result;
    };
    for (domain, id, name) in current {
        let target = gateway_tls::cert_path(domain);
        for mount in secrets.iter_mut() {
            if mount.file.as_ref().and_then(|f| f.name.as_deref()) == Some(target.as_str()) {
                result.mounted += 1;
                if mount.secret_id.as_deref() != Some(id.as_str()) {
                    mount.secret_id = Some(id.clone());
                    mount.secret_name = Some(name.clone());
                    result.changed += 1;
                }
            }
        }
    }
    result
}

/// Moves the gateway onto each domain's current certificate where it is not on
/// it yet. Every round checks this, not only a round that issued, so an update
/// that failed is made in a later one. Returns how many mounts moved.
async fn roll_out(docker: &Docker, current: &[(String, String, String)]) -> Result<usize> {
    let service = docker
        .inspect_service(GATEWAY_SERVICE, None)
        .await
        .with_context(|| format!("Cannot inspect {}", GATEWAY_SERVICE))?;
    let version = service.version.and_then(|v| v.index).unwrap_or_default();
    let mut spec = service.spec.context("The gateway service has no spec")?;
    let repointed = repoint(&mut spec, current);
    if repointed.mounted == 0 {
        bail!("the gateway mounts none of the issued certificates; is the simpled stack deployed?");
    }
    if repointed.changed == 0 {
        return Ok(0);
    }
    docker
        .update_service(
            GATEWAY_SERVICE,
            spec,
            UpdateServiceOptions {
                version: version as i32,
                ..Default::default()
            },
            None,
        )
        .await
        .context("Cannot update the gateway")?;
    Ok(repointed.changed)
}

async fn prune(docker: &Docker, domain: &str) {
    let Ok(secrets) = domain_secrets(docker, domain).await else {
        return;
    };
    for secret in stale(&secrets) {
        // Refused while a task still mounts it; the next round tries again.
        if docker.delete_secret(&secret.id).await.is_ok() {
            eprintln!("acme: removed the old certificate secret {}", secret.name);
        }
    }
}

/// Runs for the controller's lifetime.
pub async fn run(config: AcmeConfig, docker: Docker, challenges: Challenges) {
    let mut account: Option<Account> = None;
    let mut failures: HashMap<String, (u32, u64)> = HashMap::new();
    tokio::time::sleep(FIRST_CHECK_AFTER).await;
    loop {
        // Each domain's issued certificate, as (domain, secret id, secret name).
        let mut issued = Vec::new();
        for domain in &config.domains {
            let now = super::unix_now();
            if failures.get(domain).is_some_and(|(_, retry_at)| *retry_at > now) {
                continue;
            }
            let secrets = match domain_secrets(&docker, domain).await {
                Ok(secrets) => secrets,
                Err(e) => {
                    eprintln!("acme: {}: {:#}", domain, e);
                    continue;
                }
            };
            let best = current(&secrets);
            if !needs_certificate(best, now) {
                if let Some(best) = best {
                    issued.push((domain.clone(), best.id.clone(), best.name.clone()));
                }
                prune(&docker, domain).await;
                continue;
            }
            // Until a new one is issued, the gateway stays on the issued one it has.
            if let Some(best) = best.filter(|b| b.acme) {
                issued.push((domain.clone(), best.id.clone(), best.name.clone()));
            }
            if account.is_none() {
                match self::account(&config).await {
                    Ok(a) => account = Some(a),
                    Err(e) => {
                        eprintln!("acme: {:#}", e);
                        break;
                    }
                }
            }
            let Some(acct) = account.as_ref() else { break };
            eprintln!("acme: requesting a certificate for {}", domain);
            let result = async {
                let pem = issue(acct, domain, &challenges).await?;
                create_secret(&docker, domain, &pem).await
            }
            .await;
            match result {
                Ok((id, name)) => {
                    eprintln!("acme: issued {} into {}", domain, name);
                    failures.remove(domain);
                    issued.retain(|(d, _, _)| d != domain);
                    issued.push((domain.clone(), id, name));
                }
                Err(e) => {
                    let count = failures.get(domain).map_or(0, |(n, _)| *n) + 1;
                    let wait = backoff(count, jitter());
                    eprintln!(
                        "acme: {}: {:#}. Trying again in {} minutes.",
                        domain,
                        e,
                        wait.as_secs() / 60
                    );
                    failures.insert(domain.clone(), (count, now + wait.as_secs()));
                }
            }
        }
        let mut next_check = CHECK_EVERY;
        if !issued.is_empty() {
            match roll_out(&docker, &issued).await {
                Ok(0) => {}
                Ok(moved) => eprintln!("acme: rolling the gateway onto {} new certificate(s)", moved),
                Err(e) => {
                    eprintln!(
                        "acme: {:#}. Trying again in {} minutes.",
                        e,
                        ROLL_OUT_RETRY.as_secs() / 60
                    );
                    next_check = ROLL_OUT_RETRY;
                }
            }
        }
        tokio::time::sleep(next_check).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{TaskSpec, TaskSpecContainerSpec, TaskSpecContainerSpecFile, TaskSpecContainerSpecSecrets};

    fn cert(id: &str, acme: bool, not_after: u64, index: u64) -> CertSecret {
        CertSecret {
            id: id.to_string(),
            name: format!("tls_{}", id),
            acme,
            not_after,
            index,
        }
    }

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 24 * 3600;

    #[test]
    fn a_placeholder_or_a_certificate_close_to_expiry_is_replaced() {
        assert!(needs_certificate(None, NOW));
        assert!(needs_certificate(Some(&cert("p", false, 0, 1)), NOW));
        assert!(needs_certificate(Some(&cert("a", true, NOW + 29 * DAY, 2)), NOW));
        assert!(!needs_certificate(Some(&cert("a", true, NOW + 31 * DAY, 2)), NOW));
    }

    #[test]
    fn the_issued_certificate_that_lasts_longest_is_current() {
        let secrets = [
            cert("p", false, 0, 9),
            cert("a", true, NOW + 60 * DAY, 3),
            cert("b", true, NOW + 89 * DAY, 5),
        ];
        assert_eq!(current(&secrets).unwrap().id, "b");
    }

    #[test]
    fn pruning_keeps_the_two_issued_certificates_that_last_longest() {
        let secrets = [
            cert("p", false, 0, 1),
            cert("a", true, NOW, 2),
            cert("b", true, NOW + DAY, 3),
            cert("c", true, NOW + 2 * DAY, 4),
        ];
        let ids: Vec<&str> = stale(&secrets).iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["p", "a"]);
        // An imported certificate created last but expiring first goes first.
        let secrets = [
            cert("a", true, NOW + 60 * DAY, 2),
            cert("c", true, NOW + 89 * DAY, 3),
            cert("b", true, NOW + 30 * DAY, 4),
        ];
        let ids: Vec<&str> = stale(&secrets).iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
        // Without an issued certificate, the placeholder stays.
        assert!(stale(&[cert("p", false, 0, 1)]).is_empty());
    }

    #[test]
    fn backoff_doubles_from_an_hour_to_a_day() {
        assert_eq!(backoff(1, 0.0), Duration::from_secs(3600));
        assert_eq!(backoff(3, 0.0), Duration::from_secs(4 * 3600));
        assert_eq!(backoff(10, 0.0), Duration::from_secs(24 * 3600));
        assert!(backoff(1, 1.0) <= Duration::from_secs(3960));
    }

    #[test]
    fn repointing_swaps_only_the_mounts_of_renewed_domains() {
        let mount = |domain: &str, name: &str| TaskSpecContainerSpecSecrets {
            file: Some(TaskSpecContainerSpecFile {
                name: Some(gateway_tls::cert_path(domain)),
                ..Default::default()
            }),
            secret_id: Some(format!("id-{}", name)),
            secret_name: Some(name.to_string()),
        };
        let mut spec = ServiceSpec {
            task_template: Some(TaskSpec {
                container_spec: Some(TaskSpecContainerSpec {
                    secrets: Some(vec![mount("a.example.com", "old-a"), mount("b.example.com", "old-b")]),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let changed = repoint(
            &mut spec,
            &[
                ("a.example.com".to_string(), "id-new".to_string(), "new-a".to_string()),
                ("b.example.com".to_string(), "id-old-b".to_string(), "old-b".to_string()),
            ],
        );
        // b is on its current secret already.
        assert_eq!(changed, Repointed { mounted: 2, changed: 1 });
        // Once moved, a later round has nothing to roll out.
        let again = repoint(
            &mut spec,
            &[("a.example.com".to_string(), "id-new".to_string(), "new-a".to_string())],
        );
        assert_eq!(again, Repointed { mounted: 1, changed: 0 });
        let secrets = spec.task_template.unwrap().container_spec.unwrap().secrets.unwrap();
        assert_eq!(secrets[0].secret_name.as_deref(), Some("new-a"));
        assert_eq!(secrets[0].secret_id.as_deref(), Some("id-new"));
        assert_eq!(secrets[1].secret_name.as_deref(), Some("old-b"));
    }

    #[test]
    fn the_expiry_comes_from_the_first_certificate() {
        let pem = gateway_tls::self_signed("shop.example.com").unwrap();
        assert!(not_after(&pem).unwrap() > NOW);
    }
}
