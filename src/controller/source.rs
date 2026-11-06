//! Where the controller reads a deployment's services from: Swarm services on a
//! swarm, containers on standalone Docker. Both are found by the labels
//! `prepare-deployment` writes (`model::labels`).

use super::inspect::{self, LogQuery};
use super::model::{labels, ServiceState};
use anyhow::{Context, Result};
use bollard::models::{
    ContainerSummaryHealthStatusEnum, ContainerSummaryStateEnum, Service, ServiceUpdateStatusStateEnum,
};
use bollard::query_parameters::{ListContainersOptions, ListServicesOptions};
use bollard::query_parameters::{ListNodesOptions, ListTasksOptions, LogsOptions};
use bollard::Docker;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A log, as Docker streams it.
pub type LogStream = Pin<Box<dyn futures_util::Stream<Item = Result<axum::body::Bytes, std::io::Error>> + Send>>;

pub trait Source: Send + Sync {
    fn services<'a>(&'a self, deployment: &'a str) -> BoxFuture<'a, Result<Vec<ServiceState>>>;
    /// Every deployment's services, keyed by deployment: what `/health` and
    /// `/api/v1/status` read, from one listing.
    fn all_services(&self) -> BoxFuture<'_, Result<BTreeMap<String, Vec<ServiceState>>>>;

    /// A service's tasks (Swarm) or containers (standalone). `None` when the
    /// deployment has no such service.
    fn tasks<'a>(&'a self, _deployment: &'a str, _service: &'a str) -> BoxFuture<'a, Result<Option<Value>>> {
        Box::pin(async { Ok(None) })
    }
    /// A service's log. `None` when the deployment has no such service.
    fn logs<'a>(
        &'a self,
        _deployment: &'a str,
        _service: &'a str,
        _query: LogQuery,
    ) -> BoxFuture<'a, Result<Option<LogStream>>> {
        Box::pin(async { Ok(None) })
    }
    /// The gateway's tasks, or its container.
    fn gateway(&self) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async { Ok(Value::Null) })
    }
    /// The swarm's nodes; `None` on standalone Docker.
    fn nodes(&self) -> BoxFuture<'_, Result<Option<Value>>> {
        Box::pin(async { Ok(None) })
    }
    /// The gateway's desired and running tasks (Swarm) or containers (standalone).
    fn gateway_replicas(&self) -> BoxFuture<'_, Result<(u64, u64)>> {
        Box::pin(async { Ok((1, 1)) })
    }
}

/// The gateway's certificates, for `/health`.
pub trait CertSource: Send + Sync {
    /// A domain's current certificate: whether it was issued, and its expiry.
    /// `None` when the domain has none.
    fn current<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Result<Option<(bool, u64)>>>;
}

/// Domain to its current certificate: whether it was issued, and its expiry.
type CurrentCerts = HashMap<String, (bool, u64)>;

/// The one listing in flight, which every lookup that arrives meanwhile awaits.
type Listing = futures_util::future::Shared<BoxFuture<'static, Result<Arc<CurrentCerts>, Arc<anyhow::Error>>>>;

/// Every domain's current certificate from one listing of the certificate
/// secrets, kept for `CACHE_FOR`: they change at most once a round of the ACME
/// task, and `/health` may be polled often, for many domains.
pub struct CachedCerts {
    docker: Docker,
    /// Locked only to read or replace what is here, never across the listing:
    /// lookups that arrive together, as one `/health` makes for every domain,
    /// share the one listing in flight, and when it fails they all fail at once
    /// instead of each listing in turn.
    state: std::sync::Mutex<CertsState>,
}

#[derive(Default)]
struct CertsState {
    /// Shared, so each lookup takes a reference rather than a copy of the map.
    cached: Option<(std::time::Instant, Arc<CurrentCerts>)>,
    listing: Option<Listing>,
}

impl CachedCerts {
    const CACHE_FOR: std::time::Duration = std::time::Duration::from_secs(30);

    pub fn new(docker: Docker) -> Self {
        CachedCerts {
            docker,
            state: std::sync::Mutex::new(CertsState::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CertsState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn all(&self) -> Result<Arc<CurrentCerts>> {
        use futures_util::FutureExt;
        let listing = {
            let mut state = self.state();
            if let Some((at, map)) = state.cached.as_ref() {
                if at.elapsed() < Self::CACHE_FOR {
                    return Ok(map.clone());
                }
            }
            match &state.listing {
                Some(listing) => listing.clone(),
                None => {
                    let docker = self.docker.clone();
                    let listing: Listing = async move { Self::list(docker).await.map(Arc::new).map_err(Arc::new) }
                        .boxed()
                        .shared();
                    state.listing = Some(listing.clone());
                    listing
                }
            }
        };
        let result = listing.clone().await;
        let mut state = self.state();
        // The first lookup back from this listing records it; a later listing
        // another lookup started meanwhile is left alone.
        if state.listing.as_ref().is_some_and(|l| l.ptr_eq(&listing)) {
            state.listing = None;
            if let Ok(map) = &result {
                state.cached = Some((std::time::Instant::now(), map.clone()));
            }
        }
        result.map_err(|e| anyhow::anyhow!("{:#}", e))
    }

    async fn list(docker: Docker) -> Result<CurrentCerts> {
        let found = super::acme::list_cert_secrets(&docker, crate::gateway_tls::labels::DOMAIN).await?;
        let mut by_domain: HashMap<String, Vec<crate::gateway_tls::CertSecret>> = HashMap::new();
        for (domain, secret) in found {
            by_domain.entry(domain).or_default().push(secret);
        }
        Ok(by_domain
            .into_iter()
            .filter_map(|(domain, secrets)| {
                let current = crate::gateway_tls::current(&secrets).map(|c| (c.acme, c.not_after))?;
                Some((domain, current))
            })
            .collect())
    }
}

impl CertSource for CachedCerts {
    fn current<'a>(&'a self, domain: &'a str) -> BoxFuture<'a, Result<Option<(bool, u64)>>> {
        Box::pin(async move { Ok(self.all().await?.get(domain).copied()) })
    }
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
    pub fn new(docker: Docker, mode: Mode) -> Self {
        DockerSource { docker, mode }
    }
}

/// `DOCKER_HOST`, or else the local socket (the named pipe on Windows).
pub fn connect_docker() -> Result<Docker> {
    if std::env::var_os("DOCKER_HOST").is_some() {
        Docker::connect_with_defaults()
    } else {
        Docker::connect_with_local_defaults()
    }
    .context("Cannot connect to Docker")
}

/// The gateway, as the `simpled` stack and the standalone deploy.sh name it.
const GATEWAY_SWARM_SERVICE: &str = crate::deploy_steps::GATEWAY_SERVICE;
const GATEWAY_CONTAINER: &str = crate::docker_generator::GATEWAY_CONTAINER;

fn one_service_filter(deployment: &str, service: &str) -> HashMap<String, Vec<String>> {
    HashMap::from([(
        "label".to_string(),
        vec![
            format!("{}={}", labels::DEPLOYMENT, deployment),
            format!("{}={}", labels::SERVICE, service),
        ],
    )])
}

/// The container whose log a standalone service shows: a running one, else the
/// one created last. The list includes stopped containers, which would otherwise
/// be picked as readily as the live one.
fn log_container(containers: Vec<bollard::models::ContainerSummary>) -> Option<String> {
    containers
        .into_iter()
        .filter(|c| c.id.is_some())
        .max_by_key(|c| {
            (
                c.state == Some(ContainerSummaryStateEnum::RUNNING),
                c.created.unwrap_or(0),
            )
        })
        .and_then(|c| c.id)
}

fn logs_options(query: &LogQuery) -> LogsOptions {
    LogsOptions {
        follow: query.follow,
        stdout: true,
        stderr: true,
        since: query.since.clamp(0, i32::MAX as i64) as i32,
        until: 0,
        timestamps: query.timestamps,
        tail: query.tail.to_string(),
    }
}

impl DockerSource {
    /// The Swarm service ID of one of a deployment's services, found by labels,
    /// so only services the controller owns are found.
    async fn swarm_service_id(&self, deployment: &str, service: &str) -> Result<Option<String>> {
        let services = self
            .docker
            .list_services(Some(ListServicesOptions {
                filters: Some(one_service_filter(deployment, service)),
                status: None,
            }))
            .await
            .context("Cannot list the swarm's services")?;
        Ok(services.into_iter().find_map(|s| s.id))
    }

    async fn containers(
        &self,
        filters: HashMap<String, Vec<String>>,
    ) -> Result<Vec<bollard::models::ContainerSummary>> {
        self.docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await
            .context("Cannot list the containers")
    }

    async fn swarm_tasks(&self, service_id: &str) -> Result<Vec<Value>> {
        let tasks = self
            .docker
            .list_tasks(Some(ListTasksOptions {
                filters: Some(HashMap::from([("service".to_string(), vec![service_id.to_string()])])),
            }))
            .await
            .context("Cannot list the tasks")?;
        let nodes = self
            .docker
            .list_nodes(None::<ListNodesOptions>)
            .await
            .context("Cannot list the nodes")?;
        Ok(inspect::tasks_json(&tasks, &inspect::hostnames(&nodes)))
    }
}

/// Every labelled service: the filter of `all_services`.
fn any_service_filter() -> HashMap<String, Vec<String>> {
    HashMap::from([("label".to_string(), vec![labels::SERVICE.to_string()])])
}

/// The deployment a labelled object belongs to.
fn deployment_of(labels_map: &HashMap<String, String>) -> Option<&String> {
    labels_map.get(labels::DEPLOYMENT)
}

/// Standalone: one state per (deployment, service) from its containers. Every
/// labelled container is meant to be running: jobs, which exit, are not
/// labelled. One without a deployment label is nobody's, as on Swarm.
fn standalone_states(containers: &[bollard::models::ContainerSummary]) -> BTreeMap<(String, String), ServiceState> {
    let mut by_service: BTreeMap<(String, String), ServiceState> = BTreeMap::new();
    for container in containers {
        let Some(labels_map) = container.labels.as_ref() else {
            continue;
        };
        let Some((name, application, version, deployed_at)) = labelled(labels_map) else {
            continue;
        };
        let Some(deployment) = deployment_of(labels_map).cloned() else {
            continue;
        };
        let state = by_service.entry((deployment, name.clone())).or_insert(ServiceState {
            service: name,
            application,
            version,
            deployed_at,
            desired: 0,
            running: 0,
            unhealthy: 0,
            updating: false,
            image: container.image.clone().unwrap_or_default(),
        });
        state.desired += 1;
        if container.state == Some(ContainerSummaryStateEnum::RUNNING) {
            state.running += 1;
            if container.health.as_ref().and_then(|h| h.status) == Some(ContainerSummaryHealthStatusEnum::UNHEALTHY) {
                state.unhealthy += 1;
            }
        }
    }
    by_service
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
        image: spec
            .task_template
            .as_ref()
            .and_then(|t| t.container_spec.as_ref())
            .and_then(|c| c.image.as_deref())
            // Swarm pins the digest it resolved; the tag is what was deployed.
            .map(|image| image.split('@').next().unwrap_or(image).to_string())
            .unwrap_or_default(),
    })
}

impl Source for DockerSource {
    fn tasks<'a>(&'a self, deployment: &'a str, service: &'a str) -> BoxFuture<'a, Result<Option<Value>>> {
        Box::pin(async move {
            let tasks = match self.mode {
                Mode::Swarm => match self.swarm_service_id(deployment, service).await? {
                    Some(id) => self.swarm_tasks(&id).await?,
                    None => return Ok(None),
                },
                Mode::Standalone => {
                    let containers = self.containers(one_service_filter(deployment, service)).await?;
                    if containers.is_empty() {
                        return Ok(None);
                    }
                    containers.iter().map(inspect::container_json).collect()
                }
            };
            Ok(Some(
                json!({ "deployment": deployment, "service": service, "tasks": tasks }),
            ))
        })
    }

    fn logs<'a>(
        &'a self,
        deployment: &'a str,
        service: &'a str,
        query: LogQuery,
    ) -> BoxFuture<'a, Result<Option<LogStream>>> {
        Box::pin(async move {
            let options = logs_options(&query);
            let stream = match self.mode {
                // The service's log, aggregated across nodes by the manager.
                Mode::Swarm => match self.swarm_service_id(deployment, service).await? {
                    Some(id) => self.docker.service_logs(&id, Some(options)).boxed(),
                    None => return Ok(None),
                },
                Mode::Standalone => {
                    let containers = self.containers(one_service_filter(deployment, service)).await?;
                    match log_container(containers) {
                        Some(id) => self.docker.logs(&id, Some(options)).boxed(),
                        None => return Ok(None),
                    }
                }
            };
            let stream: LogStream =
                Box::pin(stream.map(|chunk| chunk.map(|o| o.into_bytes()).map_err(std::io::Error::other)));
            Ok(Some(stream))
        })
    }

    fn gateway(&self) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
            match self.mode {
                Mode::Swarm => Ok(json!({ "tasks": self.swarm_tasks(GATEWAY_SWARM_SERVICE).await? })),
                Mode::Standalone => {
                    let filters = HashMap::from([("name".to_string(), vec![format!("^/{}$", GATEWAY_CONTAINER)])]);
                    let containers = self.containers(filters).await?;
                    Ok(json!({ "tasks": containers.iter().map(inspect::container_json).collect::<Vec<_>>() }))
                }
            }
        })
    }

    fn gateway_replicas(&self) -> BoxFuture<'_, Result<(u64, u64)>> {
        Box::pin(async move {
            match self.mode {
                Mode::Swarm => {
                    let services = self
                        .docker
                        .list_services(Some(ListServicesOptions {
                            filters: Some(HashMap::from([(
                                "name".to_string(),
                                vec![GATEWAY_SWARM_SERVICE.to_string()],
                            )])),
                            status: Some(true),
                        }))
                        .await
                        .context("Cannot list the swarm's services")?;
                    // The name filter matches prefixes too.
                    let status = services
                        .iter()
                        .find(|s| s.spec.as_ref().and_then(|s| s.name.as_deref()) == Some(GATEWAY_SWARM_SERVICE))
                        .and_then(|s| s.service_status.as_ref());
                    Ok((
                        status.and_then(|s| s.desired_tasks).unwrap_or_default(),
                        status.and_then(|s| s.running_tasks).unwrap_or_default(),
                    ))
                }
                Mode::Standalone => {
                    let filters = HashMap::from([("name".to_string(), vec![format!("^/{}$", GATEWAY_CONTAINER)])]);
                    let containers = self.containers(filters).await?;
                    let running = containers
                        .iter()
                        .filter(|c| c.state == Some(ContainerSummaryStateEnum::RUNNING))
                        .count() as u64;
                    Ok((1, running))
                }
            }
        })
    }

    fn nodes(&self) -> BoxFuture<'_, Result<Option<Value>>> {
        Box::pin(async move {
            if self.mode != Mode::Swarm {
                return Ok(None);
            }
            let nodes = self
                .docker
                .list_nodes(None::<ListNodesOptions>)
                .await
                .context("Cannot list the nodes")?;
            Ok(Some(
                json!({ "nodes": nodes.iter().map(inspect::node_json).collect::<Vec<_>>() }),
            ))
        })
    }

    fn all_services(&self) -> BoxFuture<'_, Result<BTreeMap<String, Vec<ServiceState>>>> {
        Box::pin(async move {
            let mut all: BTreeMap<String, Vec<ServiceState>> = BTreeMap::new();
            match self.mode {
                Mode::Swarm => {
                    let services = self
                        .docker
                        .list_services(Some(ListServicesOptions {
                            filters: Some(any_service_filter()),
                            status: Some(true),
                        }))
                        .await
                        .context("Cannot list the swarm's services")?;
                    for service in &services {
                        let deployment = service
                            .spec
                            .as_ref()
                            .and_then(|s| s.labels.as_ref())
                            .and_then(deployment_of);
                        if let (Some(deployment), Some(state)) = (deployment, swarm_service(service)) {
                            all.entry(deployment.clone()).or_default().push(state);
                        }
                    }
                    for states in all.values_mut() {
                        states.sort_by(|a, b| a.service.cmp(&b.service));
                    }
                }
                Mode::Standalone => {
                    let containers = self.containers(any_service_filter()).await?;
                    for ((deployment, _), state) in standalone_states(&containers) {
                        all.entry(deployment).or_default().push(state);
                    }
                }
            }
            Ok(all)
        })
    }

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
                    let containers = self.containers(label_filter(deployment)).await?;
                    Ok(standalone_states(&containers).into_values().collect())
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_come_from_the_running_container() {
        let container = |id: &str, running: bool, created: i64| bollard::models::ContainerSummary {
            id: Some(id.to_string()),
            state: Some(if running {
                ContainerSummaryStateEnum::RUNNING
            } else {
                ContainerSummaryStateEnum::EXITED
            }),
            created: Some(created),
            ..Default::default()
        };
        assert_eq!(
            log_container(vec![container("dead", false, 20), container("live", true, 10)]).as_deref(),
            Some("live")
        );
        assert_eq!(
            log_container(vec![container("old", false, 10), container("new", false, 20)]).as_deref(),
            Some("new")
        );
        assert_eq!(log_container(vec![]), None);
    }
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
                image: String::new(),
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
