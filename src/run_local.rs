use crate::docker_compose::*;
use crate::resolved_spec::*;
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a compose file is written and how its project is named.
pub struct ComposeTarget {
    pub dir: PathBuf,
    pub project: String,
    /// A test run: Docker-managed volumes, no fixed container names, so it never
    /// collides with the developer's own `local_env` stack or its data.
    pub isolated: bool,
}

impl ComposeTarget {
    pub fn local(spec: &EnvironmentResolvedSpec) -> Self {
        ComposeTarget {
            dir: PathBuf::from("local_env"),
            project: local_project_name(&spec.current_deployment.application_name),
            isolated: false,
        }
    }

    pub fn test(spec: &EnvironmentResolvedSpec) -> Self {
        ComposeTarget {
            dir: PathBuf::from("test_env"),
            project: test_project_name(&spec.current_deployment.application_name),
            isolated: true,
        }
    }
}

pub fn run(spec: &EnvironmentResolvedSpec, exclude: &[String]) -> Result<()> {
    run_filtered(spec, |s| !s.excluded && !exclude.iter().any(|e| e == &s.full_name))
}

pub fn run_only_extra(spec: &EnvironmentResolvedSpec) -> Result<()> {
    run_filtered(spec, |s| !s.is_app_service && !s.excluded)
}

pub fn generate_config(spec: &EnvironmentResolvedSpec) -> Result<()> {
    write_compose(spec, &ComposeTarget::local(spec), |s| !s.excluded)
}

fn run_filtered<F>(spec: &EnvironmentResolvedSpec, filter: F) -> Result<()>
where
    F: Fn(&ServiceResolvedSpec) -> bool,
{
    let target = ComposeTarget::local(spec);
    write_compose(spec, &target, &filter)?;

    println!("Running docker compose up...");

    let status = Command::new("docker")
        .current_dir(&target.dir)
        .args(["compose", "up", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;

    if !status.success() {
        return Err(anyhow!("docker compose failed"));
    }

    Ok(())
}

/// Writes `target.dir/docker-compose.yaml` for the services `filter` keeps, plus
/// every service's env and secret files. Host-run services (`working_dir`) get
/// theirs whether or not they are in the compose file.
pub fn write_compose<F>(spec: &EnvironmentResolvedSpec, target: &ComposeTarget, filter: F) -> Result<()>
where
    F: Fn(&ServiceResolvedSpec) -> bool,
{
    let output_dir: &Path = &target.dir;
    fs::create_dir_all(output_dir).with_context(|| format!("Failed to create {:?}", output_dir))?;

    println!("Starting services for deployment: {}", spec.current_deployment.name);

    let mut services_map = HashMap::new();

    for service in spec.current_deployment.services.iter() {
        write_working_dir(service, spec)?;

        if filter(service) {
            let docker_service = prepare_service_with(service, spec, output_dir, target.isolated)?;
            services_map.insert(service.full_name.clone(), docker_service);
        }
    }

    // `docker compose up` honours depends_on, so the local run gets the same
    // ordering the deploy scripts build by hand for Swarm and standalone Docker.
    // Dependencies that are not part of this compose file (excluded services, or
    // ones run on the host) are dropped: compose rejects references to services it
    // does not know.
    let included: Vec<String> = services_map.keys().cloned().collect();
    for service in spec.current_deployment.services.iter() {
        if !included.contains(&service.full_name) {
            continue;
        }
        let mut depends_on = HashMap::new();
        for dep in &service.depends_on {
            if !included.contains(dep) {
                continue;
            }
            let Some(dep_service) = spec.current_deployment.services.iter().find(|s| &s.full_name == dep) else {
                continue;
            };
            // Waiting for `service_healthy` is only possible when the dependency
            // declares a healthcheck; otherwise compose can only order the start.
            let condition = if dep_service.healthcheck.as_ref().is_some_and(|hc| !hc.is_disabled()) {
                "service_healthy"
            } else {
                "service_started"
            };
            depends_on.insert(
                dep.clone(),
                DependsOnCondition {
                    condition: condition.to_string(),
                },
            );
        }
        if let Some(docker_service) = services_map.get_mut(&service.full_name) {
            docker_service.depends_on = depends_on;
        }
    }

    // An isolated run declares its named volumes so compose creates and, with
    // `down --volumes`, removes them.
    let volumes = if target.isolated {
        spec.current_deployment
            .volumes
            .iter()
            .map(|name| (name.clone(), DockerVolume::default()))
            .collect()
    } else {
        HashMap::new()
    };

    let compose = DockerCompose {
        name: Some(target.project.clone()),
        services: services_map,
        networks: HashMap::new(),
        volumes,
    };

    let compose_path = output_dir.join("docker-compose.yaml");
    let yaml = serde_yaml::to_string(&compose)?;
    fs::write(&compose_path, yaml)?;

    println!("Generated docker-compose.yaml at {:?}", compose_path);

    Ok(())
}
