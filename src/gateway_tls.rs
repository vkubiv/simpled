//! The gateway's certificates on Docker Swarm: one Swarm secret per domain,
//! holding the certificate chain and the key in one PEM file, which Traefik
//! reads through its file provider. Secrets reach every node through Raft, so
//! any node can run a gateway task (`docs/design/controller.md` §7.1).
//!
//! The controller issues and renews them. Until it has, a domain is served with a
//! self-signed placeholder, which `step tls-secrets` creates on the first deploy.

use crate::resolved_spec::IngressResolvedSpec;
use anyhow::{Context, Result};

/// Labels of a certificate secret. The controller never reads a certificate back:
/// what it needs to know is in these.
pub mod labels {
    pub const DOMAIN: &str = "simpled.tls.domain";
    /// `acme` for a certificate the controller issued, `self-signed` for the
    /// placeholder.
    pub const ISSUER: &str = "simpled.tls.issuer";
    /// Unix seconds; `0` for the placeholder, which nothing renews by date.
    pub const NOT_AFTER: &str = "simpled.tls.not_after";
    /// On the config holding the controller's ACME account: the directory it
    /// was created at, and the e-mail it was registered with.
    pub const ACME_DIRECTORY: &str = "simpled.acme.directory";
    pub const ACME_EMAIL: &str = "simpled.acme.email";
}

pub const ISSUER_ACME: &str = "acme";
pub const ISSUER_SELF_SIGNED: &str = "self-signed";
/// Pruning and the gateway's other objects use the same owner.
pub const OWNER_LABEL: &str = "simpled.scope=environment";
/// `${SIMPLED_TLS_<n>}` in the ingress stack: the secret to mount per domain.
pub const VAR_PREFIX: &str = "SIMPLED_TLS_";

/// A domain's certificate secret, as its labels describe it. `step tls-secrets`
/// reads these through the Docker CLI, the controller through the Docker API.
#[derive(Debug, Clone, PartialEq)]
pub struct CertSecret {
    pub id: String,
    pub name: String,
    pub acme: bool,
    pub not_after: u64,
    /// Swarm's object version: secrets are immutable, so it orders them by
    /// creation.
    pub index: u64,
}

impl CertSecret {
    /// From the issuer and expiry labels, as Docker reports them.
    pub fn from_labels(id: String, name: String, issuer: Option<&str>, not_after: Option<&str>, index: u64) -> Self {
        CertSecret {
            id,
            name,
            acme: issuer == Some(ISSUER_ACME),
            not_after: not_after.and_then(|v| v.trim().parse().ok()).unwrap_or_default(),
            index,
        }
    }

    /// The order secrets are preferred in: issued over placeholder, then the
    /// one that lasts longest, then the newest.
    fn rank(&self) -> (bool, u64, u64) {
        (self.acme, self.not_after, self.index)
    }
}

/// The secret a domain is served with: the issued certificate that lasts
/// longest, or else the newest placeholder.
pub fn current(secrets: &[CertSecret]) -> Option<&CertSecret> {
    secrets.iter().max_by_key(|s| s.rank())
}

/// The secrets in the order `current` prefers them, best first.
pub fn by_preference(secrets: &[CertSecret]) -> Vec<&CertSecret> {
    let mut sorted: Vec<&CertSecret> = secrets.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.rank()));
    sorted
}

/// The labels of a domain's certificate secret, owner included.
pub fn secret_labels(domain: &str, issuer: &str, not_after: u64) -> Vec<(String, String)> {
    let (owner_key, owner_value) = OWNER_LABEL.split_once('=').unwrap_or((OWNER_LABEL, ""));
    vec![
        (owner_key.to_string(), owner_value.to_string()),
        (labels::DOMAIN.to_string(), domain.to_string()),
        (labels::ISSUER.to_string(), issuer.to_string()),
        (labels::NOT_AFTER.to_string(), not_after.to_string()),
    ]
}

/// `secret_labels` as `docker secret create` arguments.
pub fn secret_label_args(domain: &str, issuer: &str, not_after: u64) -> Vec<String> {
    secret_labels(domain, issuer, not_after)
        .into_iter()
        .flat_map(|(k, v)| ["--label".to_string(), format!("{}={}", k, v)])
        .collect()
}

/// The domains the gateway holds a certificate for: every domain it answers on
/// over TLS, redirect sources included. Empty unless the certificates come
/// from Let's Encrypt.
pub fn domains(ingress: &IngressResolvedSpec) -> Vec<String> {
    let letsencrypt = ingress.tls.as_ref().is_some_and(|t| t.letsencrypt.is_some());
    if !letsencrypt || !ingress.terminates_tls() {
        return vec![];
    }
    let mut domains = ingress.domains.clone();
    domains.sort();
    domains.dedup();
    domains
}

/// The PEM the gateway reads for a domain: the chain, then the key, each ending
/// in a newline whatever the source gave, so Traefik finds both blocks.
pub fn certificate_pem(chain: &str, key: &str) -> String {
    format!("{}\n{}\n", chain.trim_end(), key.trim_end())
}

/// Where a gateway task finds a domain's PEM, which is both its `certFile` and
/// its `keyFile`: Go's TLS loader takes the certificates from one and the key
/// from the other, skipping the other blocks.
pub fn cert_path(domain: &str) -> String {
    format!("/run/secrets/simpled_tls/{}.pem", domain)
}

/// The base of a domain's secret names, `<base>_<hash of the content>`.
pub fn secret_base(domain: &str) -> String {
    crate::swarm_objects::object_base(&format!("tls_{}", domain))
}

/// A self-signed certificate and key for `domain`, in one PEM.
pub fn self_signed(domain: &str) -> Result<String> {
    let key = rcgen::KeyPair::generate().context("Cannot generate a key")?;
    let mut params = rcgen::CertificateParams::new(vec![domain.to_string()])
        .with_context(|| format!("'{}' is not a domain a certificate can name", domain))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, format!("{} (simpled placeholder)", domain));
    let cert = params.self_signed(&key).context("Cannot sign the placeholder")?;
    Ok(certificate_pem(&cert.pem(), &key.serialize_pem()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_holds_a_certificate_and_its_key() {
        let pem = self_signed("shop.example.com").unwrap();
        assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"), "{}", pem);
        assert!(pem.contains("-----BEGIN PRIVATE KEY-----"), "{}", pem);
    }

    #[test]
    fn the_key_starts_on_its_own_line() {
        let chain = "-----BEGIN CERTIFICATE-----\nAA\n-----END CERTIFICATE-----";
        let key = "-----BEGIN PRIVATE KEY-----\nBB\n-----END PRIVATE KEY-----\n\n";
        assert_eq!(
            certificate_pem(chain, key),
            "-----BEGIN CERTIFICATE-----\nAA\n-----END CERTIFICATE-----\n\
             -----BEGIN PRIVATE KEY-----\nBB\n-----END PRIVATE KEY-----\n"
        );
    }

    #[test]
    fn secret_names_fit_swarm() {
        let long = format!("{}.example.com", "a".repeat(80));
        let name = crate::deploy_steps::object_name(&secret_base(&long), b"pem");
        assert!(name.len() <= 64, "{}", name);
        assert!(name.starts_with("tls_"), "{}", name);
    }
}
