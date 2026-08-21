use crate::spec::*;
use anyhow::{Result, anyhow};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

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
    for config in &app_spec.configs {
        let deployment_config = deployment.configs.iter().find(|c| c.name == config.name)
             .ok_or_else(|| anyhow!("Config {} required by application is not provided by deployment {}", config.name, env_name))?;

        let mut available_files = HashSet::new();
        for file_path in &deployment_config.files {
             let path = Path::new(file_path);
             if path.is_dir() {
                 if let Ok(entries) = fs::read_dir(path) {
                     for entry in entries {
                         if let Ok(entry) = entry {
                              let path = entry.path();
                              if path.is_file() {
                                  if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                      available_files.insert(name.to_string());
                                  }
                              }
                         }
                     }
                 }
             } else {
                 if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                      available_files.insert(name.to_string());
                 }
             }
        }
        
        for required_file in &config.files {
             if !available_files.contains(required_file) {
                 return Err(anyhow!("Config {} requires file {}, but it is not provided by deployment config (checked paths: {:?})", 
                      config.name, required_file, deployment_config.files));
             }
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

    // Validate `depends_on` references and reject dependency cycles. A cycle has
    // no valid start order, and the deploy scripts walk these edges to decide what
    // must be running before a job runs.
    for service in app_spec.all_services() {
        for dep in &service.depends_on {
            if !available_services.contains(dep) {
                return Err(anyhow!("Service {} depends on {} which is not defined in application", service.name, dep));
            }
        }
    }

    if let Some(cycle) = find_depends_on_cycle(app_spec) {
        return Err(anyhow!("Dependency cycle in depends_on: {}", cycle.join(" -> ")));
    }

    validate_service_env_references(app_spec)?;

    Ok(())
}

/// Every variable a service names must be declared in the app's `environment`.
fn validate_service_env_references(app_spec: &AppSpec) -> Result<()> {
    let mut app_defined_env_vars = HashSet::new();
    for env in &app_spec.environment.external {
        app_defined_env_vars.insert(&env.name);
    }
    // Optional variables count as defined: a service may reference one directly,
    // and it is simply left off the service when the environment does not
    // provide it (see `filter_service_env_vars`).
    for env in &app_spec.environment.optional {
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

/// Depth-first search over `depends_on` edges. Returns the services on the first
/// cycle found (starting and ending on the same name), or `None` when the graph
/// is acyclic.
fn find_depends_on_cycle(app_spec: &AppSpec) -> Option<Vec<String>> {
    let mut done: HashSet<&str> = HashSet::new();

    for service in app_spec.all_services() {
        // `path` doubles as the visit stack and as the reported cycle.
        let mut path: Vec<&str> = Vec::new();
        if let Some(cycle) = visit(&service.name, app_spec, &mut path, &mut done) {
            return Some(cycle);
        }
    }

    None
}

fn visit<'a>(
    name: &'a str,
    app_spec: &'a AppSpec,
    path: &mut Vec<&'a str>,
    done: &mut HashSet<&'a str>,
) -> Option<Vec<String>> {
    if done.contains(name) {
        return None;
    }
    if let Some(pos) = path.iter().position(|n| *n == name) {
        let mut cycle: Vec<String> = path[pos..].iter().map(|n| n.to_string()).collect();
        cycle.push(name.to_string());
        return Some(cycle);
    }

    path.push(name);
    if let Some(service) = app_spec.all_services().find(|s| s.name == name) {
        for dep in &service.depends_on {
            if let Some(cycle) = visit(dep, app_spec, path, done) {
                return Some(cycle);
            }
        }
    }
    path.pop();
    done.insert(name);

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, depends_on: &[&str]) -> ServiceSpec {
        ServiceSpec {
            name: name.to_string(),
            service_type: ServiceType::Internal,
            is_app_service: true,
            image: ImageSpec::Exact(format!("org/{}", name)),
            environment: vec![],
            configs: vec![],
            secrets: vec![],
            ports: vec![],
            expose: vec![],
            volumes: vec![],
            command: None,
            entrypoint: None,
            healthcheck: None,
            depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn app_spec(app_services: Vec<ServiceSpec>) -> AppSpec {
        AppSpec {
            name: "shop".to_string(),
            version: semver::Version::new(1, 0, 0),
            environment: AppEnvironment {
                external: vec![], optional: vec![], relative: vec![], internal: vec![],
            },
            app_services,
            extra_services: vec![],
            configs: vec![],
            secrets: vec![],
            volumes: vec![],
        }
    }

    #[test]
    fn acyclic_dependencies_are_accepted() {
        let spec = app_spec(vec![
            service("api", &["migrate"]),
            service("migrate", &["primary-db"]),
            service("primary-db", &[]),
        ]);
        assert_eq!(find_depends_on_cycle(&spec), None);
    }

    #[test]
    fn a_dependency_cycle_is_reported() {
        let spec = app_spec(vec![
            service("api", &["worker"]),
            service("worker", &["api"]),
        ]);
        let cycle = find_depends_on_cycle(&spec).expect("cycle must be detected");
        assert_eq!(cycle.first(), cycle.last());
        assert!(cycle.contains(&"api".to_string()) && cycle.contains(&"worker".to_string()));
    }

    #[test]
    fn an_optional_variable_can_be_referenced_directly() {
        let mut svc = service("backend", &[]);
        svc.environment = vec![ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string())];
        let mut spec = app_spec(vec![svc]);
        spec.environment.optional = vec![OptionalEnvVariable { name: "LIVEKIT_API_KEY".to_string() }];

        validate_service_env_references(&spec).expect("optional variables are defined");
    }

    #[test]
    fn an_undeclared_variable_is_still_rejected() {
        let mut svc = service("backend", &[]);
        svc.environment = vec![ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string())];
        let spec = app_spec(vec![svc]);

        let err = validate_service_env_references(&spec).unwrap_err();
        assert!(err.to_string().contains("references undefined environment variable LIVEKIT_API_KEY"));
    }

    #[test]
    fn a_diamond_is_not_a_cycle() {
        // Both branches reach `primary-db`; revisiting a finished node must not
        // look like a cycle.
        let spec = app_spec(vec![
            service("api", &["migrate", "cache-warm"]),
            service("migrate", &["primary-db"]),
            service("cache-warm", &["primary-db"]),
            service("primary-db", &[]),
        ]);
        assert_eq!(find_depends_on_cycle(&spec), None);
    }
}
