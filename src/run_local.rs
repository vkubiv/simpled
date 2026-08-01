use crate::resolved_spec::*;
use crate::docker_compose::*;
use anyhow::{Result, Context, anyhow};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::collections::HashMap;
use serde::Serialize;



pub fn run(spec: &EnvironmentResolvedSpec, exclude: &[String]) -> Result<()> {
    run_filtered(spec, |s| !exclude.iter().any(|e| e == &s.full_name))
}

pub fn run_only_extra(spec: &EnvironmentResolvedSpec) -> Result<()> {
    run_filtered(spec, |s| !s.is_app_service)
}

pub fn generate_config(spec: &EnvironmentResolvedSpec) -> Result<()> {
    write_compose(spec, |_| true)
}

fn run_filtered<F>(spec: &EnvironmentResolvedSpec, filter: F) -> Result<()>
where
    F: Fn(&crate::resolved_spec::ServiceResolvedSpec) -> bool,
{
    write_compose(spec, &filter)?;

    let output_dir = Path::new("local_env");
    println!("Running docker compose up...");

    let status = Command::new("docker")
        .current_dir(output_dir)
        .args(&["compose", "up", "--remove-orphans"])
        .status()
        .context("Failed to run docker compose")?;

    if !status.success() {
        return Err(anyhow!("docker compose failed"));
    }

    Ok(())
}

fn write_compose<F>(spec: &EnvironmentResolvedSpec, filter: F) -> Result<()>
where
    F: Fn(&crate::resolved_spec::ServiceResolvedSpec) -> bool,
{
    let output_dir = Path::new("local_env");
    fs::create_dir_all(output_dir).context("Failed to create local_env directory")?;

    println!("Starting services for deployment: {}", spec.current_deployment.name);

    let mut services_map = HashMap::new();

    for service in spec.current_deployment.services.iter() {
        // Host-run services (working_dir set) get their `.env` and secrets even
        // when they are excluded from the generated compose (e.g. only-extra).
        write_working_dir(service, spec)?;

        if filter(service) {
            let docker_service = prepare_service(service, spec, output_dir)?;
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
            let Some(dep_service) = spec.current_deployment.services.iter()
                .find(|s| &s.full_name == dep) else { continue };
            // Waiting for `service_healthy` is only possible when the dependency
            // declares a healthcheck; otherwise compose can only order the start.
            let condition = if dep_service.healthcheck.as_ref().is_some_and(|hc| !hc.is_disabled()) {
                "service_healthy"
            } else {
                "service_started"
            };
            depends_on.insert(dep.clone(), DependsOnCondition { condition: condition.to_string() });
        }
        if let Some(docker_service) = services_map.get_mut(&service.full_name) {
            docker_service.depends_on = depends_on;
        }
    }

    let compose = DockerCompose {
        services: services_map,
        networks: HashMap::new(),
    };

    let compose_path = output_dir.join("docker-compose.yaml");
    let yaml = serde_yaml::to_string(&compose)?;
    fs::write(&compose_path, yaml)?;

    println!("Generated docker-compose.yaml at {:?}", compose_path);

    Ok(())
}

