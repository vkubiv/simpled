use crate::spec::*;
use anyhow::{Result, anyhow};
use std::collections::HashSet;

pub fn validate(env_spec: &DeploymentEnvironmentSpec, app_spec: &AppSpec, env_name: &str) -> Result<()> {
    let deployment = env_spec.deployments.iter()
        .find(|d| d.name == env_name)
        .ok_or_else(|| anyhow!("Deployment {} not found in envspec", env_name))?;

    // Check application name
    if deployment.application.name != app_spec.name {
         return Err(anyhow!("Deployment {} expects application {}, but appspec is for {}", 
             env_name, deployment.application.name, app_spec.name));
    }

    // Check version
    if let Some(req) = &deployment.application.version {
        if !req.matches(&app_spec.version) {
             return Err(anyhow!("App version {} does not satisfy deployment requirement {}", 
                 app_spec.version, req));
        }
    }

    // Check environment variables
    let provided_env_vars: HashSet<&String> = deployment.environment.iter().map(|e| &e.name).collect();
    let mut missing_env_vars = Vec::new();
    for env_var in &app_spec.environment.external {
        if !provided_env_vars.contains(&env_var.name) && env_var.default.is_none() {
             missing_env_vars.push(&env_var.name);
        }
    }

    if !missing_env_vars.is_empty() {
        return Err(anyhow!("Environment variables {:?} required by application are not provided by deployment {}", 
             missing_env_vars, env_name));
    }

    // Check secrets
    let provided_secrets: HashSet<&String> = deployment.secrets.iter().map(|c| &c.secret_name).collect();
    for secret in &app_spec.secrets {
        if !provided_secrets.contains(&secret.secret_name) {
             return Err(anyhow!("Secret {} required by application is not provided by deployment {}", 
                 secret.secret_name, env_name));
        }
    }
    
    // Check configs
    // TODO: check if all required files are present
    let provided_configs: HashSet<&String> = deployment.configs.iter().map(|c| &c.name).collect();
    for config in &app_spec.configs {
        if !provided_configs.contains(&config.name) {
            return Err(anyhow!("Config {} required by application is not provided by deployment {}", 
                 config.name, env_name));
        }
    }

    // Check services
    // Identify all available services in app (app_services + extra_services)
    let mut available_services = HashSet::new();
    for svc in app_spec.all_services() {
        available_services.insert(&svc.name);
    }

    if let Some(services) = &deployment.services {
        for (svc_name, _) in services {
            if !available_services.contains(svc_name) {
                 return Err(anyhow!("Deployment configures service {} which is not defined in application", svc_name));
            }
        }
    }

    // Validate service environment variable references
    let mut app_defined_env_vars = HashSet::new();
    for env in &app_spec.environment.external {
        app_defined_env_vars.insert(&env.name);
    }
    for env in &app_spec.environment.relative {
        app_defined_env_vars.insert(&env.name);
    }
    for env in &app_spec.environment.internal {
        app_defined_env_vars.insert(&env.name);
    }

    for service in app_spec.all_services() {
        for env_opt in &service.environment {
            if let ServiceEnvOption::Simple(var_name) = env_opt {
                if !app_defined_env_vars.contains(var_name) {
                     return Err(anyhow!("Service {} references undefined environment variable {}", service.name, var_name));
                }
            }
        }
    }

    Ok(())
}
