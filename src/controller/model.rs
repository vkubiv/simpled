//! What the controller answers, computed from the state of a deployment's services.
//! Pure functions, so the rules are tested without Docker.

use serde::Serialize;
use serde_json::{json, Value};

/// Labels `prepare-deployment` puts on every service of a deployment. The controller
/// finds a deployment's services by them and reads its version from them.
pub mod labels {
    pub const DEPLOYMENT: &str = "simpled.deployment";
    pub const APPLICATION: &str = "simpled.application";
    pub const VERSION: &str = "simpled.version";
    pub const DEPLOYED_AT: &str = "simpled.deployed_at";
    /// The name from the spec (`api`), not the Swarm name (`prod_api`). Only a
    /// deployment's own services carry it: the gateway also has
    /// `simpled.deployment`, naming the deployment that deployed it.
    pub const SERVICE: &str = "simpled.service";

    /// Set by deploy.sh when it runs, so `deployed_at` is the time of the deploy
    /// and the stack file is the same every time the deployment is prepared.
    pub const DEPLOYED_AT_VAR: &str = "SIMPLED_DEPLOYED_AT";

    /// The labels of one of a deployment's services. Only the time comes in as a
    /// variable, written for the shell or for `docker stack deploy`, which both
    /// read `${...}`.
    pub fn for_service(
        deployment: &crate::resolved_spec::DeploymentResolvedSpec,
        service: &str,
    ) -> std::collections::BTreeMap<String, String> {
        [
            (DEPLOYMENT, deployment.name.clone()),
            (APPLICATION, deployment.application_name.clone()),
            (VERSION, deployment.application_version.clone()),
            (DEPLOYED_AT, format!("${{{}}}", DEPLOYED_AT_VAR)),
            (SERVICE, service.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }

    /// A deployment's services the controller reports on: the long-running ones.
    /// Jobs finish, and the nginx version sidecar is not part of the app.
    pub fn is_reported(
        deployment: &crate::resolved_spec::DeploymentResolvedSpec,
        service: &crate::resolved_spec::ServiceResolvedSpec,
    ) -> bool {
        !matches!(service.service_type, crate::spec::ServiceType::Job)
            && service.full_name != crate::version_endpoint::service_name(&deployment.name)
    }
}

/// One service of a deployment, as Docker reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceState {
    pub service: String,
    pub application: String,
    pub version: String,
    pub deployed_at: u64,
    /// Tasks (Swarm) or containers (standalone) that should be running.
    pub desired: u64,
    pub running: u64,
    /// Running, but failing their healthcheck.
    pub unhealthy: u64,
    /// A Swarm update or rollback is in progress or paused.
    pub updating: bool,
    /// The image the service runs, as Swarm or Docker reports it.
    pub image: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Healthy,
    Degraded,
    Down,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Health {
    pub status: Status,
    /// The rollout has finished: no update is in progress, and every service is
    /// at the deployment's latest version.
    pub converged: bool,
    pub version: Option<String>,
    /// Swarm with `tls.letsencrypt`: the worst certificate of the deployment's
    /// domains. Anything but `valid` makes a healthy deployment `degraded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<Tls>,
}

/// The state of a domain's certificate, best first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tls {
    Valid,
    /// Issued, but within 7 days of expiry: renewal, which starts at 30 days,
    /// has kept failing.
    Expiring,
    /// The placeholder, or no certificate at all: browsers warn.
    SelfSigned,
}

const EXPIRING_WITHIN_SECONDS: u64 = 7 * 24 * 3600;

/// One domain's certificate state, from its current secret.
pub fn tls_state(current: Option<(bool, u64)>, now: u64) -> Tls {
    match current {
        Some((true, not_after)) if not_after > now + EXPIRING_WITHIN_SECONDS => Tls::Valid,
        Some((true, _)) => Tls::Expiring,
        _ => Tls::SelfSigned,
    }
}

/// Adds the certificates' state to a deployment's health: the worst one, which
/// degrades a healthy deployment. A deployment that is down stays down.
pub fn with_tls(mut health: Health, states: &[Tls]) -> Health {
    let Some(worst) = states.iter().copied().max() else {
        return health;
    };
    if worst != Tls::Valid && health.status == Status::Healthy {
        health.status = Status::Degraded;
    }
    health.tls = Some(worst);
    health
}

/// The services of the last deploy: the one with the newest `deployed_at`.
fn latest(services: &[ServiceState]) -> Option<&ServiceState> {
    services.iter().max_by_key(|s| s.deployed_at)
}

/// The `/version` document, the same shape the nginx sidecar served. `None` when
/// the deployment has no services running under its labels.
pub fn version_document(deployment: &str, services: &[ServiceState]) -> Option<Value> {
    let latest = latest(services)?;
    Some(json!({
        "application": latest.application,
        "version": latest.version,
        "deployment": deployment,
        "deployed_at": latest.deployed_at,
    }))
}

/// `down` when a service that should run has nothing running, `degraded` when one
/// runs fewer tasks than it should or has an unhealthy one. A service scaled to
/// zero on purpose counts as neither. A deployment with no services is `down`.
pub fn health(services: &[ServiceState]) -> Health {
    let Some(latest) = latest(services) else {
        return Health {
            status: Status::Down,
            converged: false,
            version: None,
            tls: None,
        };
    };
    let status = if services.iter().any(|s| s.desired > 0 && s.running == 0) {
        Status::Down
    } else if services.iter().any(|s| s.running < s.desired || s.unhealthy > 0) {
        Status::Degraded
    } else {
        Status::Healthy
    };
    let converged = services.iter().all(|s| !s.updating && s.version == latest.version);
    Health {
        status,
        converged,
        version: Some(latest.version.clone()),
        tls: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, version: &str, desired: u64, running: u64) -> ServiceState {
        ServiceState {
            service: name.to_string(),
            application: "shop".to_string(),
            version: version.to_string(),
            deployed_at: 100,
            desired,
            running,
            unhealthy: 0,
            updating: false,
            image: String::new(),
        }
    }

    #[test]
    fn every_service_running_is_healthy_and_converged() {
        let health = health(&[service("api", "1.2.3", 2, 2), service("web", "1.2.3", 1, 1)]);
        assert_eq!(
            health,
            Health {
                status: Status::Healthy,
                converged: true,
                version: Some("1.2.3".to_string()),
                tls: None,
            }
        );
    }

    #[test]
    fn a_service_with_nothing_running_is_down() {
        let health = health(&[service("api", "1.2.3", 2, 0), service("web", "1.2.3", 1, 1)]);
        assert_eq!(health.status, Status::Down);
    }

    #[test]
    fn missing_or_unhealthy_tasks_are_degraded() {
        assert_eq!(health(&[service("api", "1.2.3", 2, 1)]).status, Status::Degraded);
        let mut unhealthy = service("api", "1.2.3", 1, 1);
        unhealthy.unhealthy = 1;
        assert_eq!(health(&[unhealthy]).status, Status::Degraded);
    }

    #[test]
    fn a_service_scaled_to_zero_does_not_count() {
        let health = health(&[service("api", "1.2.3", 1, 1), service("worker", "1.2.3", 0, 0)]);
        assert_eq!(health.status, Status::Healthy);
    }

    #[test]
    fn no_services_is_down() {
        assert_eq!(
            health(&[]),
            Health {
                status: Status::Down,
                converged: false,
                version: None,
                tls: None,
            }
        );
        assert_eq!(version_document("prod", &[]), None);
    }

    #[test]
    fn an_update_in_progress_is_not_converged() {
        let mut api = service("api", "1.2.3", 1, 1);
        api.updating = true;
        assert!(!health(&[api]).converged);
    }

    #[test]
    fn the_latest_deploy_names_the_version() {
        // A service Swarm rolled back gets the previous spec back, labels
        // included, so it reports the older version and the deployment is not
        // converged.
        let mut new = service("api", "1.3.0", 1, 1);
        new.deployed_at = 200;
        let old = service("web", "1.2.3", 1, 1);
        let health = health(&[old.clone(), new.clone()]);
        assert_eq!(health.version.as_deref(), Some("1.3.0"));
        assert!(!health.converged);
        assert_eq!(
            version_document("prod", &[old, new]),
            Some(json!({"application": "shop", "version": "1.3.0", "deployment": "prod", "deployed_at": 200}))
        );
    }

    #[test]
    fn a_certificate_close_to_expiry_or_a_placeholder_degrades_a_healthy_deployment() {
        const NOW: u64 = 1_800_000_000;
        const DAY: u64 = 24 * 3600;
        assert_eq!(tls_state(Some((true, NOW + 20 * DAY)), NOW), Tls::Valid);
        assert_eq!(tls_state(Some((true, NOW + 6 * DAY)), NOW), Tls::Expiring);
        assert_eq!(tls_state(Some((false, 0)), NOW), Tls::SelfSigned);
        assert_eq!(tls_state(None, NOW), Tls::SelfSigned);

        let healthy = health(&[service("api", "1.2.3", 1, 1)]);
        let valid = with_tls(healthy.clone(), &[Tls::Valid, Tls::Valid]);
        assert_eq!((valid.status, valid.tls), (Status::Healthy, Some(Tls::Valid)));
        let expiring = with_tls(healthy.clone(), &[Tls::Valid, Tls::Expiring]);
        assert_eq!((expiring.status, expiring.tls), (Status::Degraded, Some(Tls::Expiring)));
        // Without certificates to report, nothing changes.
        assert_eq!(with_tls(healthy.clone(), &[]), healthy);
        // Down stays down.
        let down = with_tls(health(&[service("api", "1.2.3", 1, 0)]), &[Tls::SelfSigned]);
        assert_eq!(down.status, Status::Down);
        assert_eq!(
            serde_json::to_value(&down).unwrap()["tls"],
            serde_json::json!("self-signed")
        );
    }
}
