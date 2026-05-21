use crate::resolved_spec::*;
use crate::docker_compose::*;
use anyhow::{Result, Context, anyhow};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::collections::HashMap;
use serde::Serialize;



pub fn run(spec: &EnvironmentResolvedSpec) -> Result<()> {
    run_filtered(spec, |_| true)
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

    for service in spec.current_deployment.services.iter().filter(|s| filter(s)) {
        let docker_service = prepare_service(service, spec, output_dir)?;
        services_map.insert(service.full_name.clone(), docker_service);
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

