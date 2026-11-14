use crate::spec::*;
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

pub fn validate(env_spec: &DeploymentEnvironmentSpec, app_spec: &AppSpec, env_name: &str) -> Result<()> {
    let deployment = env_spec.deployment(env_name)?;

    // Check application name
    if deployment.application.name != app_spec.name {
        return Err(anyhow!(
            "Deployment {} expects application {}, but appspec is for {}",
            env_name,
            deployment.application.name,
            app_spec.name
        ));
    }

    // Check version
    if let Some(req) = &deployment.application.version {
        if !req.matches(&app_spec.version) {
            return Err(anyhow!(
                "App version {} does not satisfy deployment requirement {}",
                app_spec.version,
                req
            ));
        }
    }

    let upstream = env_spec.ingress.tls.as_ref().is_some_and(|t| t.upstream);
    if upstream && env_spec.ingress.trusted_proxies.is_empty() {
        eprintln!(
            "Warning: gateway tls.upstream is set but gateway.trusted_proxies is empty: the gateway discards the \
             X-Forwarded-For and X-Forwarded-Proto the load balancer sends, so the apps see the balancer's address \
             and plain http. List the balancer's addresses (e.g. the VPC CIDR) under gateway.trusted_proxies."
        );
    }

    // Docker targets convert the quantities themselves, so a value they cannot
    // read is reported here rather than when the files are generated.
    if env_spec.env_type != DeploymentEnvType::K8S {
        crate::docker_resources::DockerResources::from_spec(
            &deployment.defaults,
            &format!("Deployment {} defaults", env_name),
        )?;
        let mut names: Vec<&String> = deployment.services.keys().collect();
        names.sort();
        for name in names {
            crate::docker_resources::DockerResources::from_spec(
                &deployment.services[name].resources,
                &format!("Service '{}'", name),
            )?;
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
        return Err(anyhow!(
            "Environment variables {:?} required by application are not provided by deployment {}",
            missing_env_vars,
            env_name
        ));
    }

    for warning in ignored_environment_values(deployment, app_spec, &env_spec.env_type) {
        eprintln!("Warning: {}", warning);
    }

    // Check secrets
    let provided_secrets: HashSet<&String> = deployment.secrets.iter().map(|c| &c.secret_name).collect();
    for secret in &app_spec.secrets {
        if !secret.optional && !provided_secrets.contains(&secret.secret_name) {
            return Err(anyhow!(
                "Secret {} required by application is not provided by deployment {}",
                secret.secret_name,
                env_name
            ));
        }
    }

    for warning in unused_deployment_secrets(deployment, app_spec, &env_spec.env_type) {
        eprintln!("Warning: {}", warning);
    }

    // Check configs
    for config in &app_spec.configs {
        let deployment_config = deployment
            .configs
            .iter()
            .find(|c| c.name == config.name)
            .ok_or_else(|| {
                anyhow!(
                    "Config {} required by application is not provided by deployment {}",
                    config.name,
                    env_name
                )
            })?;

        let mut available_files = HashSet::new();
        for file_path in &deployment_config.files {
            let path = Path::new(file_path);
            if path.is_dir() {
                if let Ok(entries) = fs::read_dir(path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                available_files.insert(name.to_string());
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
                return Err(anyhow!(
                    "Config {} requires file {}, but it is not provided by deployment config (checked paths: {:?})",
                    config.name,
                    required_file,
                    deployment_config.files
                ));
            }
        }
    }

    // Check services
    // Identify all available services in app (app_services + extra_services)
    let mut available_services = HashSet::new();
    for svc in app_spec.all_services() {
        available_services.insert(&svc.name);
    }

    let services = &deployment.services;
    for svc_name in services.keys() {
        if !available_services.contains(svc_name) {
            return Err(anyhow!(
                "Deployment configures service {} which is not defined in application",
                svc_name
            ));
        }
    }

    for svc_name in &deployment.exclude_services {
        if !available_services.contains(svc_name) {
            return Err(anyhow!(
                "Deployment {} excludes service {} which is not defined in application",
                env_name,
                svc_name
            ));
        }
    }

    // Validate `depends_on` references and reject dependency cycles. A cycle has
    // no valid start order, and the deploy scripts walk these edges to decide what
    // must be running before a job runs. The edges are the deployment's: its
    // per-service `depends_on` replaces the app's where it is set.
    let edges = effective_depends_on(app_spec, deployment);
    for (name, deps) in &edges {
        let overridden = deployment.services.get(*name).is_some_and(|s| s.depends_on.is_some());
        for dep in *deps {
            if !available_services.contains(dep) {
                return Err(if overridden {
                    anyhow!(
                        "Deployment {} makes service {} depend on {} which is not defined in application",
                        env_name,
                        name,
                        dep
                    )
                } else {
                    anyhow!(
                        "Service {} depends on {} which is not defined in application",
                        name,
                        dep
                    )
                });
            }
        }
    }

    if let Some(cycle) = find_depends_on_cycle(&edges) {
        return Err(anyhow!("Dependency cycle in depends_on: {}", cycle.join(" -> ")));
    }

    validate_service_env_references(app_spec)?;

    Ok(())
}

/// Deployment `environment:` values no service receives: `relative` and
/// `internal` variables are computed from the app spec, and an undeclared name
/// is read by nothing. Locally an undeclared name can still feed a testspec
/// suite, so it is only reported elsewhere.
fn ignored_environment_values(
    deployment: &DeploymentSpec,
    app_spec: &AppSpec,
    env_type: &DeploymentEnvType,
) -> Vec<String> {
    let env = &app_spec.environment;
    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    for var in &deployment.environment {
        let name = &var.name;
        if !seen.insert(name) {
            continue;
        }
        let computed = if env.relative.iter().any(|r| &r.name == name) {
            Some("relative")
        } else if env.internal.iter().any(|i| &i.name == name) {
            Some("internal")
        } else {
            None
        };
        if let Some(kind) = computed {
            warnings.push(format!(
                "deployment '{}' sets {}, but the app spec declares it as {} and computes its value, \
                 so the env spec value is ignored. Declare {} as external or optional in the app spec \
                 to set it from the env spec.",
                deployment.name, name, kind, name
            ));
        } else if *env_type != DeploymentEnvType::Local
            && !env.external.iter().any(|e| &e.name == name)
            && !env.optional.iter().any(|o| &o.name == name)
        {
            warnings.push(format!(
                "deployment '{}' sets {}, which the app spec does not declare, so the value is ignored. \
                 Declare {} as external or optional in the app spec, or remove it from the env spec.",
                deployment.name, name, name
            ));
        }
    }
    warnings
}

/// Deployment `secrets:` nothing reads: not named by a `$secret(...)`
/// reference, and either not declared by the app spec or an optional secret no
/// service of the deployment mounts. Usually a misspelling, and since an optional secret may be left out, one that would
/// otherwise deploy with the feature silently off. Locally such a secret can
/// still feed a testspec suite, so it is only reported elsewhere.
fn unused_deployment_secrets(
    deployment: &DeploymentSpec,
    app_spec: &AppSpec,
    env_type: &DeploymentEnvType,
) -> Vec<String> {
    if *env_type == DeploymentEnvType::Local {
        return Vec::new();
    }
    let provided: HashSet<&String> = deployment.secrets.iter().map(|s| &s.secret_name).collect();
    let mut omitted: Vec<&str> = app_spec
        .secrets
        .iter()
        .filter(|s| s.optional && !provided.contains(&s.secret_name))
        .map(|s| s.secret_name.as_str())
        .collect();
    omitted.sort();
    omitted.dedup();

    let mut seen = HashSet::new();
    let mut warnings = Vec::new();
    for secret in &deployment.secrets {
        let name = &secret.secret_name;
        if !seen.insert(name) {
            continue;
        }
        let reference = format!("$secret({})", name);
        if deployment
            .environment
            .iter()
            .chain(&deployment.undockerized_environment)
            .any(|var| var.value.contains(&reference))
        {
            continue;
        }
        let mut warning = match app_spec.secrets.iter().find(|s| &s.secret_name == name) {
            None => format!(
                "deployment '{}' provides secret {}, which the app spec does not declare, so nothing reads it. \
                 Declare {} under secrets in the app spec, or remove it from the env spec.",
                deployment.name, name, name
            ),
            // A required secret has to be listed whether or not a service
            // mounts it, so there is nothing the env spec could change.
            Some(declared) if !declared.optional => continue,
            Some(_) => {
                let mounted = app_spec
                    .all_services()
                    .filter(|svc| !deployment.exclude_services.contains(&svc.name))
                    .any(|svc| svc.secrets.iter().any(|s| &s.name == name));
                if mounted {
                    continue;
                }
                format!(
                    "deployment '{}' provides the optional secret {}, but none of its services mounts it, so \
                     nothing reads it. Mount {} on a service in the app spec, or remove it from the env spec.",
                    deployment.name, name, name
                )
            }
        };
        if !omitted.is_empty() {
            warning.push_str(&format!(
                " If the name is misspelled: the deployment leaves out the app's optional secret{} {}.",
                if omitted.len() == 1 { "" } else { "s" },
                omitted.join(", ")
            ));
        }
        warnings.push(warning);
    }
    warnings
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
                    return Err(anyhow!(
                        "Service {} references undefined environment variable {}",
                        service.name,
                        var_name
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Depth-first search over `depends_on` edges. Returns the services on the first
/// cycle found (starting and ending on the same name), or `None` when the graph
/// is acyclic.
fn find_depends_on_cycle(edges: &DependsOnEdges<'_>) -> Option<Vec<String>> {
    let mut done: HashSet<&str> = HashSet::new();

    let mut names: Vec<&str> = edges.keys().copied().collect();
    names.sort_unstable();
    for name in names {
        // `path` doubles as the visit stack and as the reported cycle.
        let mut path: Vec<&str> = Vec::new();
        if let Some(cycle) = visit(name, edges, &mut path, &mut done) {
            return Some(cycle);
        }
    }

    None
}

/// Service name -> the services it must wait for, as one deployment sees them.
type DependsOnEdges<'a> = BTreeMap<&'a str, &'a [String]>;

/// The `depends_on` of every service in `deployment`: the deployment's own list
/// where it sets one, the app spec's otherwise, empty where neither does.
fn effective_depends_on<'a>(app_spec: &'a AppSpec, deployment: &'a DeploymentSpec) -> DependsOnEdges<'a> {
    app_spec
        .all_services()
        .map(|service| {
            let deps = deployment
                .services
                .get(&service.name)
                .and_then(|s| s.depends_on.as_deref())
                .or(service.depends_on.as_deref())
                .unwrap_or(&[]);
            (service.name.as_str(), deps)
        })
        .collect()
}

fn visit<'a>(
    name: &'a str,
    edges: &DependsOnEdges<'a>,
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
    if let Some(deps) = edges.get(name) {
        for dep in *deps {
            if let Some(cycle) = visit(dep, edges, path, done) {
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
            depends_on: (!depends_on.is_empty()).then(|| depends_on.iter().map(|d| d.to_string()).collect()),
        }
    }

    /// The app spec's own edges, as a deployment that overrides nothing sees them.
    fn edges(spec: &AppSpec) -> DependsOnEdges<'_> {
        spec.all_services()
            .map(|s| (s.name.as_str(), s.depends_on.as_deref().unwrap_or(&[])))
            .collect()
    }

    fn app_spec(app_services: Vec<ServiceSpec>) -> AppSpec {
        AppSpec {
            name: "shop".to_string(),
            version: semver::Version::new(1, 0, 0),
            environment: AppEnvironment {
                external: vec![],
                optional: vec![],
                relative: vec![],
                internal: vec![],
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
        assert_eq!(find_depends_on_cycle(&edges(&spec)), None);
    }

    #[test]
    fn a_dependency_cycle_is_reported() {
        let spec = app_spec(vec![service("api", &["worker"]), service("worker", &["api"])]);
        let cycle = find_depends_on_cycle(&edges(&spec)).expect("cycle must be detected");
        assert_eq!(cycle.first(), cycle.last());
        assert!(cycle.contains(&"api".to_string()) && cycle.contains(&"worker".to_string()));
    }

    #[test]
    fn an_optional_variable_can_be_referenced_directly() {
        let mut svc = service("backend", &[]);
        svc.environment = vec![ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string())];
        let mut spec = app_spec(vec![svc]);
        spec.environment.optional = vec![OptionalEnvVariable {
            name: "LIVEKIT_API_KEY".to_string(),
        }];

        validate_service_env_references(&spec).expect("optional variables are defined");
    }

    #[test]
    fn an_undeclared_variable_is_still_rejected() {
        let mut svc = service("backend", &[]);
        svc.environment = vec![ServiceEnvOption::Simple("LIVEKIT_API_KEY".to_string())];
        let spec = app_spec(vec![svc]);

        let err = validate_service_env_references(&spec).unwrap_err();
        assert!(err
            .to_string()
            .contains("references undefined environment variable LIVEKIT_API_KEY"));
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
        assert_eq!(find_depends_on_cycle(&edges(&spec)), None);
    }

    mod full_spec {
        //! `validate` on specs written as YAML, one rule per test.
        use super::*;
        use crate::test_support::{app_spec, env_spec};
        use std::fs;

        const APP: &str = r#"
name: shop
version: 1.2.3
environment:
  external:
    - LOG_LEVEL=info
app_services:
  api:
    type: public
    image: myorg/api
    ports:
      - "80:8080"
"#;

        /// A k8s env spec whose `prod` deployment gets `body` appended (already
        /// indented four spaces).
        fn k8s_env(body: &str) -> String {
            format!(
                r#"
type: k8s
gateway:
  hosts:
    web: shop.example.com
  tls:
    disable: true
registry:
  myorg: registry.example.com
deployments:
  prod:
    primary_host: web
    application:
      name: shop
{body}
    services:
      api:
        host: web
        prefix: /
"#
            )
        }

        fn check(app: &str, env_body: &str) -> Result<()> {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(&k8s_env(env_body), root.path());
            validate(&env, &app_spec(app), "prod")
        }

        #[test]
        fn a_matching_pair_validates() {
            check(APP, "").unwrap();
        }

        #[test]
        fn the_application_name_must_match() {
            let err = check(&APP.replace("name: shop", "name: other"), "")
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("expects application shop, but appspec is for other"),
                "{err}"
            );
        }

        #[test]
        fn the_version_requirement_must_be_satisfied() {
            let err = check(APP, "      version: \"^2.0\"").unwrap_err().to_string();
            assert!(
                err.contains("1.2.3 does not satisfy deployment requirement ^2.0"),
                "{err}"
            );
        }

        #[test]
        fn an_external_variable_without_default_must_be_provided() {
            let app = APP.replace("    - LOG_LEVEL=info", "    - DB_URL");
            let err = check(&app, "").unwrap_err().to_string();
            assert!(
                err.contains("[\"DB_URL\"] required by application are not provided"),
                "{err}"
            );

            check(&app, "    environment:\n      - DB_URL=postgres://db").unwrap();
        }

        #[test]
        fn a_declared_secret_must_be_provided() {
            let app = format!("{APP}secrets:\n  - db_password\n");
            let err = check(&app, "").unwrap_err().to_string();
            assert!(err.contains("Secret db_password required by application"), "{err}");

            check(&app, "    secrets:\n      db_password: literal").unwrap();
        }

        #[test]
        fn an_optional_secret_may_be_left_out() {
            let app = format!("{APP}secrets:\n  sendgrid_apikey:\n    optional: true\n  db_password:\n");
            check(&app, "    secrets:\n      db_password: literal").unwrap();
            check(
                &app,
                "    secrets:\n      db_password: literal\n      sendgrid_apikey: key",
            )
            .unwrap();

            let err = check(&app, "    secrets:\n      sendgrid_apikey: key")
                .unwrap_err()
                .to_string();
            assert!(err.contains("Secret db_password required by application"), "{err}");
        }

        #[test]
        fn a_config_group_and_each_of_its_files_must_be_provided() {
            let app = format!("{APP}configs:\n  data:\n    - settings.json\n");
            let err = check(&app, "").unwrap_err().to_string();
            assert!(err.contains("Config data required by application"), "{err}");

            // The group exists but holds the wrong file.
            let root = tempfile::tempdir().unwrap();
            fs::create_dir_all(root.path().join("data")).unwrap();
            fs::write(root.path().join("data").join("other.json"), "{}").unwrap();
            let env = env_spec(&k8s_env("    configs:\n      data: ./data"), root.path());
            let err = validate(&env, &app_spec(&app), "prod").unwrap_err().to_string();
            assert!(err.contains("Config data requires file settings.json"), "{err}");

            fs::write(root.path().join("data").join("settings.json"), "{}").unwrap();
            let env = env_spec(&k8s_env("    configs:\n      data: ./data"), root.path());
            validate(&env, &app_spec(&app), "prod").unwrap();
        }

        #[test]
        fn a_deployment_cannot_configure_a_service_the_app_lacks() {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(
                &k8s_env("").replace("    services:\n", "    services:\n      ghost:\n        host: web\n"),
                root.path(),
            );
            let err = validate(&env, &app_spec(APP), "prod").unwrap_err().to_string();
            assert!(err.contains("configures service ghost which is not defined"), "{err}");
        }

        #[test]
        fn depends_on_must_name_a_known_service() {
            let app = format!("{APP}    depends_on:\n      - nope\n");
            let err = check(&app, "").unwrap_err().to_string();
            assert!(err.contains("api depends on nope which is not defined"), "{err}");
        }

        /// The env for `prod` with `api` given the deployment-side `depends_on` in `deps`.
        fn env_with_api_depends_on(deps: &str) -> String {
            k8s_env("").replace(
                "        prefix: /\n",
                &format!("        prefix: /\n        depends_on: {deps}\n"),
            )
        }

        #[test]
        fn a_deployment_depends_on_must_name_a_known_service_too() {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(&env_with_api_depends_on("[nope]"), root.path());
            let err = validate(&env, &app_spec(APP), "prod").unwrap_err().to_string();
            assert!(
                err.contains("Deployment prod makes service api depend on nope which is not defined"),
                "{err}"
            );
        }

        #[test]
        fn a_deployment_depends_on_replaces_the_apps_for_validation() {
            // The app names a service only some environments run in the stack;
            // a deployment without it says so with an empty list and validates.
            let app = format!("{APP}    depends_on:\n      - primary-db\n");
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(&env_with_api_depends_on("[]"), root.path());
            validate(&env, &app_spec(&app), "prod").unwrap();
        }

        #[test]
        fn a_cycle_through_a_deployment_depends_on_is_rejected() {
            let app =
                format!("{APP}  worker:\n    type: internal\n    image: myorg/worker\n    depends_on:\n      - api\n");
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(&env_with_api_depends_on("[worker]"), root.path());
            let err = validate(&env, &app_spec(&app), "prod").unwrap_err().to_string();
            assert!(err.contains("Dependency cycle in depends_on"), "{err}");
        }

        const APP_WITH_EVERY_KIND: &str = r#"
name: shop
version: 1.2.3
environment:
  external:
    - LOG_LEVEL=info
  optional:
    - SENTRY_DSN
  relative:
    - GLITCHTIP_DOMAIN=/
  internal:
    - PORT=8080
app_services:
  api:
    type: public
    image: myorg/api
    environment: [all]
    ports:
      - "80:8080"
"#;

        fn ignored(env_yaml: &str) -> Vec<String> {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(env_yaml, root.path());
            let deployment = env.deployment("prod").unwrap();
            ignored_environment_values(deployment, &app_spec(APP_WITH_EVERY_KIND), &env.env_type)
        }

        #[test]
        fn a_relative_variable_set_by_the_env_spec_is_reported() {
            let warnings = ignored(&k8s_env(
                "    environment:\n      - GLITCHTIP_DOMAIN=https://monitor.example.com",
            ));
            assert_eq!(
                warnings,
                ["deployment 'prod' sets GLITCHTIP_DOMAIN, but the app spec declares it as relative and computes its \
                  value, so the env spec value is ignored. Declare GLITCHTIP_DOMAIN as external or optional in \
                  the app spec to set it from the env spec."]
            );
        }

        #[test]
        fn an_internal_variable_set_by_the_env_spec_is_reported() {
            let warnings = ignored(&k8s_env("    environment:\n      - PORT=9090"));
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("sets PORT, but the app spec declares it as internal"));
        }

        #[test]
        fn an_undeclared_variable_set_by_the_env_spec_is_reported() {
            let warnings = ignored(&k8s_env("    environment:\n      - FEATURE_X=on"));
            assert_eq!(
                warnings,
                [
                    "deployment 'prod' sets FEATURE_X, which the app spec does not declare, so the value is ignored. \
                  Declare FEATURE_X as external or optional in the app spec, or remove it from the env spec."
                ]
            );
        }

        #[test]
        fn external_and_optional_variables_are_not_reported() {
            let warnings = ignored(&k8s_env(
                "    environment:\n      - LOG_LEVEL=debug\n      - SENTRY_DSN=https://key@sentry",
            ));
            assert!(warnings.is_empty(), "{:?}", warnings);
        }

        #[test]
        fn a_local_deployment_may_set_undeclared_variables_for_test_suites() {
            let warnings = ignored(
                r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  prod:
    primary_host: web
    application:
      name: shop
    environment:
      - E2E_ADMIN_TOKEN=t
      - PORT=9090
"#,
            );
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("sets PORT"));
        }

        const APP_WITH_SECRETS: &str = "secrets:\n  db_password:\n  sendgrid_apikey:\n    optional: true\n";

        fn unused_secrets(app: &str, env_yaml: &str) -> Vec<String> {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(env_yaml, root.path());
            let deployment = env.deployment("prod").unwrap();
            unused_deployment_secrets(deployment, &app_spec(app), &env.env_type)
        }

        #[test]
        fn a_misspelled_optional_secret_is_reported() {
            let warnings = unused_secrets(
                &format!("{APP}{APP_WITH_SECRETS}"),
                &k8s_env("    secrets:\n      db_password: pw\n      sendgrid_api_key: key"),
            );
            assert_eq!(
                warnings,
                [
                    "deployment 'prod' provides secret sendgrid_api_key, which the app spec does not declare, so \
                     nothing reads it. Declare sendgrid_api_key under secrets in the app spec, or remove it from the \
                     env spec. If the name is misspelled: the deployment leaves out the app's optional secret \
                     sendgrid_apikey."
                ]
            );
        }

        #[test]
        fn an_undeclared_secret_gets_no_hint_when_no_optional_secret_is_left_out() {
            let warnings = unused_secrets(
                &format!("{APP}    secrets:\n      - sendgrid_apikey\n{APP_WITH_SECRETS}"),
                &k8s_env("    secrets:\n      db_password: pw\n      sendgrid_apikey: key\n      stray: x"),
            );
            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].contains("provides secret stray"), "{:?}", warnings);
            assert!(!warnings[0].contains("misspelled"), "{:?}", warnings);
        }

        #[test]
        fn declared_and_referenced_secrets_are_not_reported() {
            // `pg_password` only feeds a `$secret(...)` reference.
            let warnings = unused_secrets(
                &format!("{APP}{APP_WITH_SECRETS}"),
                &k8s_env(
                    "    environment:\n      - DB_URL=postgres://u:$secret(pg_password)@db\n    secrets:\n      \
                     db_password: pw\n      pg_password: pg",
                ),
            );
            assert!(warnings.is_empty(), "{:?}", warnings);
        }

        #[test]
        fn an_optional_secret_no_service_mounts_is_reported() {
            // Two optional secrets, and the deployment lists the one `api` does
            // not mount: the wrong name is a declared one.
            let app = format!(
                "{APP}    secrets:\n      - sendgrid_apikey\nsecrets:\n  db_password:\n  sendgrid_apikey:\n    \
                 optional: true\n  sendgrid_apikey_v2:\n    optional: true\n"
            );
            let warnings = unused_secrets(
                &app,
                &k8s_env("    secrets:\n      db_password: pw\n      sendgrid_apikey_v2: key"),
            );
            assert_eq!(
                warnings,
                [
                    "deployment 'prod' provides the optional secret sendgrid_apikey_v2, but none of its services \
                     mounts it, so nothing reads it. Mount sendgrid_apikey_v2 on a service in the app spec, or \
                     remove it from the env spec. If the name is misspelled: the deployment leaves out the app's \
                     optional secret sendgrid_apikey."
                ]
            );

            // The one `api` mounts is fine, and so is the required one nobody mounts.
            let warnings = unused_secrets(
                &app,
                &k8s_env("    secrets:\n      db_password: pw\n      sendgrid_apikey: key"),
            );
            assert!(warnings.is_empty(), "{:?}", warnings);
        }

        #[test]
        fn a_local_deployment_may_provide_undeclared_secrets_for_test_suites() {
            let warnings = unused_secrets(
                &format!("{APP}{APP_WITH_SECRETS}"),
                r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  prod:
    primary_host: web
    application:
      name: shop
    secrets:
      db_password: pw
      e2e_admin_token: t
"#,
            );
            assert!(warnings.is_empty(), "{:?}", warnings);
        }

        #[test]
        fn an_unknown_deployment_is_reported_with_the_known_ones() {
            let root = tempfile::tempdir().unwrap();
            let env = env_spec(&k8s_env(""), root.path());
            let err = validate(&env, &app_spec(APP), "staging").unwrap_err().to_string();
            assert!(err.contains("'staging' not found") && err.contains("prod"), "{err}");
        }
    }

    #[test]
    fn an_excluded_service_must_exist() {
        let spec = app_spec(vec![service("api", &[])]);
        let env = crate::test_support::env_spec(
            r#"
type: local
gateway:
  hosts:
    web: localhost:8080
deployments:
  infra:
    primary_host: web
    application:
      name: shop
    exclude_services: [api, worker]
    services:
      api:
        host: web
        prefix: /
        ports:
          - "8080:80"
"#,
            std::path::Path::new("."),
        );
        let err = validate(&env, &spec, "infra").unwrap_err().to_string();
        assert_eq!(
            err,
            "Deployment infra excludes service worker which is not defined in application"
        );
    }
}
