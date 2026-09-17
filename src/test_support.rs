#![allow(dead_code)]

//! Builders shared by the unit tests.
//!
//! Specs are written as YAML, the way users write them, and go through the real
//! transform layer, so a test exercises the same path a spec file does. The
//! resolved-spec builders fill every field with a neutral value so a test only
//! has to set what it is about.

use crate::resolved_spec::{DeploymentResolvedSpec, EnvironmentResolvedSpec, IngressResolvedSpec, ServiceResolvedSpec};
use crate::spec::{AppSpec, DeploymentEnvType, DeploymentEnvironmentSpec, ResourceLimits, ResourcesSpec, ServiceType};
use crate::spec_yaml::{AppSpecYaml, DeploymentEnvironmentSpecYaml};
use crate::transform;
use anyhow::Result;
use std::path::Path;

pub fn try_app_spec(raw: &str) -> Result<AppSpec> {
    let yaml: AppSpecYaml = serde_yaml::from_str(raw)?;
    transform::convert_app_spec(yaml, None)
}

pub fn app_spec(raw: &str) -> AppSpec {
    try_app_spec(raw).expect("app spec converts")
}

pub fn try_env_spec(raw: &str, root: &Path) -> Result<DeploymentEnvironmentSpec> {
    let yaml: DeploymentEnvironmentSpecYaml = serde_yaml::from_str(raw)?;
    transform::convert_env_spec(yaml, root, None)
}

pub fn env_spec(raw: &str, root: &Path) -> DeploymentEnvironmentSpec {
    try_env_spec(raw, root).expect("env spec converts")
}

pub fn resources() -> ResourcesSpec {
    ResourcesSpec {
        replicas: 1,
        requests: ResourceLimits {
            memory: "128Mi".to_string(),
            cpu: "100m".to_string(),
        },
        limits: ResourceLimits {
            memory: "256Mi".to_string(),
            cpu: "200m".to_string(),
        },
    }
}

pub fn resolved_service(name: &str, service_type: ServiceType, depends_on: &[&str]) -> ServiceResolvedSpec {
    ServiceResolvedSpec {
        service_type,
        is_app_service: true,
        full_name: name.to_string(),
        image: format!("registry.example.com/{}:1.0.0", name),
        environment_variables: vec![],
        undockerized_environment_variables: vec![],
        configs: vec![],
        secrets: vec![],
        ports: vec![],
        expose: vec![],
        volumes: vec![],
        command: None,
        entrypoint: None,
        healthcheck: None,
        depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
        resources: resources(),
        working_dir: None,
    }
}

pub fn resolved_deployment(services: Vec<ServiceResolvedSpec>) -> DeploymentResolvedSpec {
    DeploymentResolvedSpec {
        name: "prod".to_string(),
        application_name: "shop".to_string(),
        configs: vec![],
        secrets: vec![],
        services,
        volumes: vec![],
    }
}

pub fn resolved_env(env_type: DeploymentEnvType, deployment: DeploymentResolvedSpec) -> EnvironmentResolvedSpec {
    EnvironmentResolvedSpec {
        env_type,
        ingress: IngressResolvedSpec {
            name: "gateway".to_string(),
            tls: None,
            domains: vec![],
            rules: vec![],
            redirects: vec![],
        },
        current_deployment: deployment,
    }
}
