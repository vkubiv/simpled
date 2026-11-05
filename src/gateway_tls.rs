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
}

pub const ISSUER_ACME: &str = "acme";
pub const ISSUER_SELF_SIGNED: &str = "self-signed";
/// Pruning and the gateway's other objects use the same owner.
pub const OWNER_LABEL: &str = "simpled.scope=environment";
/// `${SIMPLED_TLS_<n>}` in the ingress stack: the secret to mount per domain.
pub const VAR_PREFIX: &str = "SIMPLED_TLS_";

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
    Ok(format!("{}{}", cert.pem(), key.serialize_pem()))
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
    fn secret_names_fit_swarm() {
        let long = format!("{}.example.com", "a".repeat(80));
        let name = crate::deploy_steps::object_name(&secret_base(&long), b"pem");
        assert!(name.len() <= 64, "{}", name);
        assert!(name.starts_with("tls_"), "{}", name);
    }
}
