//! Where the daemon reads a deployment's services from: Swarm services on a
//! swarm, containers on standalone Docker. Both are found by the labels
//! `prepare-deployment` writes (`model::labels`).

use super::model::{labels, ServiceState};
use anyhow::{Context, Result};
use bollard::models::{
    ContainerSummaryHealthStatusEnum, ContainerSummaryStateEnum, Service, ServiceUpdateStatusStateEnum,
};
use bollard::query_parameters::{ListContainersOptions, ListServicesOptions};
use bollard::Docker;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait Source: Send + Sync {
    fn services<'a>(&'a self, deployment: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Swarm,
    Standalone,
}

pub struct DockerSource {
    docker: Docker,
    mode: Mode,
}

impl DockerSource {
    /// `DOCKER_HOST`, or else the local socket (the named pipe on Windows).
    pub fn connect(mode: Mode) -> Result<Self> {
        let docker = if std::env::var_os("DOCKER_HOST").is_some() {
            Docker::connect_with_defaults()
        } else {
            Docker::connect_with_local_defaults()
        }
        .context("Cannot connect to Docker")?;
        Ok(DockerSource { docker, mode })
    }
}

fn label_filter(deployment: &str) -> HashMap<String, Vec<String>> {
    HashMap::from([(
        "label".to_string(),
        vec![
            format!("{}={}", labels::DEPLOYMENT, deployment),
            labels::SERVICE.to_string(),
        ],
    )])
}

/// The spec-level facts of a service, from its labels. `None` for anything not
/// labelled as a deployment's service.
fn labelled(labels_map: &HashMap<String, String>) -> Option<(String, String, String, u64)> {
    Some((
        labels_map.get(labels::SERVICE)?.clone(),
        labels_map.get(labels::APPLICATION).cloned().unwrap_or_default(),
        labels_map.get(labels::VERSION).cloned().unwrap_or_default(),
        labels_map
            .get(labels::DEPLOYED_AT)
            .and_then(|v| v.parse().ok())
            .unwrap_or_default(),
    ))
}

/// One Swarm service. Jobs are left out: a finished migration is not a service
/// that should be running.
pub fn swarm_service(service: &Service) -> Option<ServiceState> {
    let spec = service.spec.as_ref()?;
    if let Some(mode) = &spec.mode {
        if mode.replicated_job.is_some() || mode.global_job.is_some() {
            return None;
        }
    }
    let (name, application, version, deployed_at) = labelled(spec.labels.as_ref()?)?;
    let status = service.service_status.as_ref();
    let updating = matches!(
        service.update_status.as_ref().and_then(|u| u.state),
        Some(
            ServiceUpdateStatusStateEnum::UPDATING
                | ServiceUpdateStatusStateEnum::PAUSED
                | ServiceUpdateStatusStateEnum::ROLLBACK_STARTED
                | ServiceUpdateStatusStateEnum::ROLLBACK_PAUSED
        )
    );
    Some(ServiceState {
        service: name,
        application,
        version,
        deployed_at,
        desired: status.and_then(|s| s.desired_tasks).unwrap_or_default(),
        // With a healthcheck, Swarm only counts a task as running once it is
        // healthy, and replaces one that turns unhealthy.
        running: status.and_then(|s| s.running_tasks).unwrap_or_default(),
        unhealthy: 0,
        updating,
    })
}

impl Source for DockerSource {
    fn services<'a>(&'a self, deployment: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>> {
        Box::pin(async move {
            match self.mode {
                Mode::Swarm => {
                    let services = self
                        .docker
                        .list_services(Some(ListServicesOptions {
                            filters: Some(label_filter(deployment)),
                            status: Some(true),
                        }))
                        .await
                        .context("Cannot list the swarm's services")?;
                    let mut states: Vec<_> = services.iter().filter_map(swarm_service).collect();
                    states.sort_by(|a, b| a.service.cmp(&b.service));
                    Ok(states)
                }
                Mode::Standalone => {
                    let containers = self
                        .docker
                        .list_containers(Some(ListContainersOptions {
                            all: true,
                            filters: Some(label_filter(deployment)),
                            ..Default::default()
                        }))
                        .await
                        .context("Cannot list the containers")?;
                    // Every labelled container is meant to run: deploy.sh starts
                    // each with a restart policy, and jobs are not labelled.
                    let mut by_service: BTreeMap<String, ServiceState> = BTreeMap::new();
                    for container in &containers {
                        let Some((name, application, version, deployed_at)) =
                            container.labels.as_ref().and_then(labelled)
                        else {
                            continue;
                        };
                        let state = by_service.entry(name.clone()).or_insert(ServiceState {
                            service: name,
                            application,
                            version,
                            deployed_at,
                            desired: 0,
                            running: 0,
                            unhealthy: 0,
                            updating: false,
                        });
                        state.desired += 1;
                        if container.state == Some(ContainerSummaryStateEnum::RUNNING) {
                            state.running += 1;
                            if container.health.as_ref().and_then(|h| h.status)
                                == Some(ContainerSummaryHealthStatusEnum::UNHEALTHY)
                            {
                                state.unhealthy += 1;
                            }
                        }
                    }
                    Ok(by_service.into_values().collect())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{
        ServiceServiceStatus, ServiceSpec, ServiceSpecMode, ServiceSpecModeReplicatedJob, ServiceUpdateStatus,
    };

    fn service(labels_map: &[(&str, &str)], mode: Option<ServiceSpecMode>) -> Service {
        Service {
            spec: Some(ServiceSpec {
                labels: Some(labels_map.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
                mode,
                ..Default::default()
            }),
            service_status: Some(ServiceServiceStatus {
                running_tasks: Some(1),
                desired_tasks: Some(2),
                completed_tasks: None,
            }),
            ..Default::default()
        }
    }

    const LABELS: [(&str, &str); 5] = [
        ("simpled.deployment", "prod"),
        ("simpled.service", "api"),
        ("simpled.application", "shop"),
        ("simpled.version", "1.2.3"),
        ("simpled.deployed_at", "1790945826"),
    ];

    #[test]
    fn a_swarm_service_is_read_from_its_labels_and_status() {
        let mut api = service(&LABELS, None);
        api.update_status = Some(ServiceUpdateStatus {
            state: Some(ServiceUpdateStatusStateEnum::UPDATING),
            ..Default::default()
        });
        assert_eq!(
            swarm_service(&api),
            Some(ServiceState {
                service: "api".to_string(),
                application: "shop".to_string(),
                version: "1.2.3".to_string(),
                deployed_at: 1790945826,
                desired: 2,
                running: 1,
                unhealthy: 0,
                updating: true,
            })
        );
    }

    #[test]
    fn jobs_and_the_gateway_are_not_a_deployments_services() {
        let job = service(
            &LABELS,
            Some(ServiceSpecMode {
                replicated_job: Some(ServiceSpecModeReplicatedJob::default()),
                ..Default::default()
            }),
        );
        assert_eq!(swarm_service(&job), None);
        // The gateway names who deployed it, but is no deployment's service.
        let gateway = service(&[("simpled.deployment", "prod"), ("simpled.version", "2.1.0")], None);
        assert_eq!(swarm_service(&gateway), None);
    }
}
