//! End-to-end checks of `simpled prepare-deployment`: a small application and
//! environment are written to a temporary directory, the real binary is run on
//! them, and the generated deployment is inspected. This is what catches a
//! generator that silently drops a spec field or writes an object the target
//! would reject.

mod common;

use common::{envspec, prepare, read, STAND_IN_BINARY};
use std::fs;
use std::path::Path;

/// The `docker service create` arguments of a job, from the deployment's plan.
fn job_args(out: &Path, job: &str) -> Vec<String> {
    let plan: serde_json::Value = serde_json::from_str(&read(&out.join("prod").join("plan.json"))).unwrap();
    plan["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["name"] == job)
        .unwrap_or_else(|| panic!("no job {} in {}", job, plan))["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

#[test]
fn kubernetes_manifests_cover_every_kind_of_service() {
    let dir = prepare(&envspec("k8s", "", ""));
    let out = dir.path().join("manifests");

    // Long-running services become a Deployment and, with ports, a Service.
    let api = read(&out.join("deployment-api.yaml"));
    assert!(api.contains("kind: Deployment"), "{}", api);
    assert!(api.contains("image: registry.example.com/myorg/api:1.2.3"), "{}", api);
    assert!(api.contains("  replicas: 2\n"), "{}", api);
    assert!(api.contains("value: \"https://shop.example.com\""), "{}", api);
    assert!(out.join("service-api.yaml").exists());
    assert!(out.join("deployment-primary-db.yaml").exists());

    // A job is a Job, without a Service.
    let job = read(&out.join("job-migrate.yaml"));
    assert!(job.contains("kind: Job"), "{}", job);
    assert!(!out.join("deployment-migrate.yaml").exists());
    assert!(!out.join("service-migrate.yaml").exists());

    // The documented `db_password` spelling produces a name kubectl accepts, and
    // the reference from the pod matches it.
    let secret = read(&out.join("secret-shop-db-password.yaml"));
    assert!(secret.contains("  name: shop-db-password"), "{}", secret);
    assert!(api.contains("              name: shop-db-password"), "{}", api);
    assert!(out.join("configmap-shop-data.yaml").exists());

    // Every manifest parses as YAML.
    for entry in fs::read_dir(&out).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "yaml") {
            for doc in read(&path).split("\n---\n") {
                serde_yaml::from_str::<serde_yaml::Value>(doc)
                    .unwrap_or_else(|e| panic!("{} is not valid YAML: {}", path.display(), e));
            }
        }
    }

    let ingress = read(&out.join("ingress.yaml"));
    assert!(ingress.contains("- host: shop.example.com"), "{}", ingress);
    assert!(ingress.contains("            name: api\n"), "{}", ingress);
}

#[test]
fn standalone_docker_reaches_services_through_traefik() {
    let dir = prepare(&envspec("docker", "", "type: traefik"));
    let out = dir.path().join("docker-deploy");

    let script = read(&out.join("deploy.sh"));
    // Traefik dials `<deployment>_<service>`, so the container must answer to it.
    assert!(script.contains("--network-alias prod_api"), "{}", script);
    let traefik = read(&out.join("traefik").join("dynamic_conf.yml"));
    assert!(traefik.contains("url: \"http://prod_api:80/\""), "{}", traefik);

    // Job phases: the database, then the migration in the foreground, then api.
    let db = script.find("docker run -d --name primary-db").unwrap();
    let wait = script.find("wait_healthy primary-db").unwrap();
    let job = script.find("docker run --name migrate").unwrap();
    let api = script.find("docker run -d --name api").unwrap();
    assert!(db < wait && wait < job && job < api, "{}", script);

    assert_eq!(read(&out.join("secrets").join("shop-db_password")), "s3cr3t");
}

#[test]
fn swarm_stack_carries_replicas_and_keeps_jobs_out() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");

    let stack = read(&out.join("prod").join("docker-compose.yaml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
    assert_eq!(
        parsed["services"]["api"]["deploy"]["replicas"].as_u64(),
        Some(2),
        "{}",
        stack
    );
    assert!(
        parsed["services"]["migrate"].is_null(),
        "jobs are run by the script: {}",
        stack
    );

    let script = read(&out.join("deploy.sh"));
    assert!(
        script.contains("./simpled step run-job prod/plan.json 'migrate'"),
        "{}",
        script
    );
    // The binary that runs the steps ships with the deployment.
    assert_eq!(read(&out.join("simpled")), STAND_IN_BINARY);
}

/// The fixture env spec with deployment-wide `defaults.resources` and an `api`
/// override of its own.
fn envspec_with_resources(env_type: &str, extra_top_level: &str, gateway_type: &str) -> String {
    envspec(env_type, extra_top_level, gateway_type).replace(
        "    services:\n      api:\n",
        r#"    defaults:
      resources:
        limits:
          memory: 512Mi
    services:
      api:
        resources:
          requests:
            memory: 256Mi
            cpu: 250m
          limits:
            memory: 1Gi
            cpu: "2"
"#,
    )
}

#[test]
fn swarm_stack_and_jobs_are_limited_by_their_resources() {
    let dir = prepare(&envspec_with_resources("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");

    let stack = read(&out.join("prod").join("docker-compose.yaml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
    let api = &parsed["services"]["api"]["deploy"];
    assert_eq!(api["replicas"].as_u64(), Some(2), "{}", stack);
    assert_eq!(api["resources"]["limits"]["memory"].as_str(), Some("1g"), "{}", stack);
    assert_eq!(api["resources"]["limits"]["cpus"].as_str(), Some("2"), "{}", stack);
    assert_eq!(
        api["resources"]["reservations"]["memory"].as_str(),
        Some("256m"),
        "{}",
        stack
    );
    assert_eq!(
        api["resources"]["reservations"]["cpus"].as_str(),
        Some("0.25"),
        "{}",
        stack
    );

    // A service without an override takes the deployment defaults, and only
    // what they set.
    let db = &parsed["services"]["primary-db"]["deploy"]["resources"];
    assert_eq!(db["limits"]["memory"].as_str(), Some("512m"), "{}", stack);
    assert!(db["limits"]["cpus"].is_null(), "{}", stack);
    assert!(db["reservations"].is_null(), "{}", stack);

    let args = job_args(&out, "migrate");
    assert!(has_pair(&args, "--limit-memory", "512m"), "{:?}", args);
    assert!(
        !args.iter().any(|a| a == "--limit-cpu" || a.starts_with("--reserve-")),
        "{:?}",
        args
    );
}

#[test]
fn standalone_docker_limits_containers_by_their_resources() {
    let dir = prepare(&envspec_with_resources("docker", "", "type: traefik"));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));

    let api = script
        .lines()
        .find(|l| l.starts_with("docker run -d --name api "))
        .unwrap();
    assert!(
        api.contains(" --memory 1g --cpus 2 --memory-reservation 256m "),
        "{}",
        api
    );
    let job = script
        .lines()
        .find(|l| l.starts_with("docker run --name migrate "))
        .unwrap();
    assert!(job.contains(" --memory 512m "), "{}", job);
    assert!(!job.contains("--cpus"), "{}", job);
}

#[test]
fn without_resources_docker_containers_are_unconstrained() {
    let swarm = prepare(&envspec("docker", "swarm_mode: true", ""));
    let stack = read(
        &swarm
            .path()
            .join("docker-deploy")
            .join("prod")
            .join("docker-compose.yaml"),
    );
    assert!(!stack.contains("resources"), "{}", stack);
    let script = read(&swarm.path().join("docker-deploy").join("deploy.sh"));
    assert!(
        !script.contains("--limit-") && !script.contains("--reserve-"),
        "{}",
        script
    );

    let standalone = prepare(&envspec("docker", "", "type: traefik"));
    let script = read(&standalone.path().join("docker-deploy").join("deploy.sh"));
    assert!(!script.contains("--memory") && !script.contains("--cpus"), "{}", script);
}

#[test]
fn an_unreadable_quantity_fails_before_anything_is_generated() {
    let env = envspec_with_resources("docker", "swarm_mode: true", "").replace("memory: 1Gi", "memory: 1GB");
    let dir = tempfile::tempdir().unwrap();
    common::write_project(dir.path(), &env);
    let output = common::simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Service 'api': resources.limits.memory '1GB'"),
        "{}",
        stderr
    );
}

#[test]
fn kubernetes_keeps_writing_the_builtin_defaults() {
    let dir = prepare(&envspec_with_resources("k8s", "", ""));
    let out = dir.path().join("manifests");

    let api = read(&out.join("deployment-api.yaml"));
    assert!(
        api.contains("            memory: 256Mi\n            cpu: 250m\n"),
        "{}",
        api
    );
    assert!(api.contains("            memory: 1Gi\n            cpu: 2\n"), "{}", api);
    // Defaults set only a memory limit; the rest is the built-in 128Mi / 100m.
    let db = read(&out.join("deployment-primary-db.yaml"));
    assert!(
        db.contains("          requests:\n            memory: 128Mi\n            cpu: 100m\n"),
        "{}",
        db
    );
    assert!(
        db.contains("          limits:\n            memory: 512Mi\n            cpu: 100m\n"),
        "{}",
        db
    );
}

fn host_mode_ports(stack: &str, service: &str) -> Vec<u64> {
    let parsed: serde_yaml::Value = serde_yaml::from_str(stack).unwrap();
    let ports = parsed["services"][service]["ports"].as_sequence().expect("ports list");
    ports
        .iter()
        .map(|p| {
            assert_eq!(p["mode"].as_str(), Some("host"), "{}", stack);
            assert_eq!(p["protocol"].as_str(), Some("tcp"), "{}", stack);
            assert_eq!(p["target"].as_u64(), p["published"].as_u64(), "{}", stack);
            p["published"].as_u64().unwrap()
        })
        .collect()
}

/// The swarm gateway publishes in host mode, so the routing mesh does not
/// replace the client address with its own.
#[test]
fn swarm_gateway_publishes_its_ports_in_host_mode() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let stack = read(
        &dir.path()
            .join("docker-deploy")
            .join("ingress")
            .join("docker-compose.yaml"),
    );
    assert_eq!(host_mode_ports(&stack, "traefik"), vec![80]);
    assert!(!stack.contains("start-first"), "{}", stack);

    let env = envspec(
        "docker",
        "swarm_mode: true",
        "type: traefik\n  trusted_proxies:\n    - 10.0.0.0/8\n    - 2600:9000::/28",
    )
    .replace("    disable: true", "    letsencrypt:\n      email: ops@example.com");
    let dir = prepare(&env);
    let ingress = dir.path().join("docker-deploy").join("ingress");
    let stack = read(&ingress.join("docker-compose.yaml"));
    assert_eq!(host_mode_ports(&stack, "traefik"), vec![80, 443]);

    let traefik = read(&ingress.join("traefik").join("traefik.yml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&traefik).unwrap();
    for entrypoint in ["web", "websecure"] {
        assert_eq!(
            parsed["entryPoints"][entrypoint]["forwardedHeaders"]["trustedIPs"][1].as_str(),
            Some("2600:9000::/28"),
            "{}",
            traefik
        );
    }
}

#[test]
fn trusted_proxies_reach_the_standalone_gateway() {
    let dir = prepare(&envspec(
        "docker",
        "",
        "type: traefik\n  trusted_proxies: [\"10.0.0.0/8\"]",
    ));
    let out = dir.path().join("docker-deploy");
    let traefik = read(&out.join("traefik").join("traefik.yml"));
    assert!(
        traefik.contains(
            "  web:\n    address: \":80\"\n    forwardedHeaders:\n      trustedIPs:\n        - \"10.0.0.0/8\"\n"
        ),
        "{}",
        traefik
    );
    // Standalone publishing is untouched: no mesh in front of `docker run -p`.
    assert!(read(&out.join("deploy.sh")).contains(" -p 80:80"));
}

#[test]
fn an_invalid_trusted_proxy_fails_validation() {
    let env = envspec("docker", "", "type: traefik\n  trusted_proxies: [\"10.0.0.0/40\"]");
    let dir = tempfile::tempdir().unwrap();
    common::write_project(dir.path(), &env);
    let output = common::simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid gateway trusted_proxies entry '10.0.0.0/40'"),
        "{}",
        stderr
    );
}

/// Runs `prepare-deployment prod` on `env`, expecting it to fail, and returns stderr.
fn prepare_fails(env: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    common::write_project(dir.path(), env);
    let output = common::simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(
        !output.status.success(),
        "expected a failure:\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The fixture env spec with TLS ended in front of the gateway.
fn upstream_envspec(env_type: &str, extra_top_level: &str, gateway: &str) -> String {
    envspec(env_type, extra_top_level, gateway).replace("    disable: true", "    upstream: true")
}

#[test]
fn an_upstream_tls_swarm_gateway_serves_plain_http_on_every_node() {
    let env = upstream_envspec(
        "docker",
        "swarm_mode: true",
        "type: traefik\n  global: true\n  placement:\n    constraints: [\"node.role == worker\"]\n  trusted_proxies: [\"10.0.0.0/16\"]\n  redirects:\n    - from: example.com\n      to: shop.example.com",
    );
    let dir = prepare(&env);
    let ingress = dir.path().join("docker-deploy").join("ingress");

    let traefik = read(&ingress.join("traefik").join("traefik.yml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&traefik).unwrap();
    let web = &parsed["entryPoints"]["web"];
    assert_eq!(web["address"].as_str(), Some(":80"), "{}", traefik);
    // No redirect to https: behind an HTTP listener it would loop.
    assert!(web["http"].is_null(), "{}", traefik);
    assert_eq!(
        web["forwardedHeaders"]["trustedIPs"][0].as_str(),
        Some("10.0.0.0/16"),
        "{}",
        traefik
    );
    assert!(parsed["entryPoints"]["websecure"].is_null(), "{}", traefik);
    assert!(parsed["certificatesResolvers"].is_null(), "{}", traefik);
    assert_eq!(parsed["ping"]["entryPoint"].as_str(), Some("web"), "{}", traefik);

    let dynamic = read(&ingress.join("traefik").join("dynamic_conf.yml"));
    serde_yaml::from_str::<serde_yaml::Value>(&dynamic).unwrap();
    assert!(!dynamic.contains("websecure"), "{}", dynamic);
    assert!(!dynamic.contains("tls:"), "{}", dynamic);
    assert!(!dynamic.contains("certResolver"), "{}", dynamic);
    // Clients still arrive over https, so a redirect keeps that scheme.
    assert!(
        dynamic.contains("replacement: \"https://shop.example.com/${1}\""),
        "{}",
        dynamic
    );

    let stack = read(&ingress.join("docker-compose.yaml"));
    assert_eq!(host_mode_ports(&stack, "traefik"), vec![80]);
    let stack: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
    let deploy = &stack["services"]["traefik"]["deploy"];
    assert_eq!(deploy["mode"].as_str(), Some("global"));
    assert_eq!(
        deploy["placement"]["constraints"][0].as_str(),
        Some("node.role == worker")
    );
    assert!(!ingress.parent().unwrap().join("letsencrypt").exists());
}

#[test]
fn an_upstream_tls_standalone_traefik_publishes_only_port_80() {
    let dir = prepare(&upstream_envspec(
        "docker",
        "",
        "type: traefik\n  trusted_proxies: [\"10.0.0.0/16\"]",
    ));
    let out = dir.path().join("docker-deploy");
    let script = read(&out.join("deploy.sh"));
    let traefik_run = script
        .lines()
        .find(|l| l.starts_with("docker run -d --name traefik-ingress"))
        .unwrap();
    assert!(traefik_run.contains(" -p 80:80"), "{}", traefik_run);
    assert!(!traefik_run.contains("443"), "{}", traefik_run);
    let traefik = read(&out.join("traefik").join("traefik.yml"));
    assert!(
        traefik.contains("ping:\n  entryPoint: web\n  manualRouting: true\n"),
        "{}",
        traefik
    );
    assert!(!traefik.contains("websecure"), "{}", traefik);
}

#[test]
fn upstream_tls_without_trusted_proxies_warns() {
    let dir = tempfile::tempdir().unwrap();
    common::write_project(
        dir.path(),
        &upstream_envspec("docker", "swarm_mode: true", "type: traefik"),
    );
    let output = common::simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{}", stderr);
    assert!(
        stderr.contains("Warning: gateway tls.upstream is set but gateway.trusted_proxies is empty"),
        "{}",
        stderr
    );
}

#[test]
fn gateway_tls_and_mode_combinations_that_cannot_work_fail_validation() {
    let letsencrypt = "    letsencrypt:\n      email: ops@example.com";
    let cases = [
        (
            upstream_envspec("docker", "swarm_mode: true", "type: traefik")
                .replace("    upstream: true", &format!("    upstream: true\n{}", letsencrypt)),
            "tls.upstream cannot be combined with tls.letsencrypt or tls.secret",
        ),
        (
            envspec("docker", "swarm_mode: true", "type: nginx"),
            "gateway type nginx is no longer supported",
        ),
        (
            envspec("docker", "", "").replace("    disable: true", "    secret: shop-tls"),
            "gateway tls.secret cannot be set for Docker environment",
        ),
        (
            upstream_envspec("k8s", "", ""),
            "gateway tls.upstream cannot be set for K8S environment",
        ),
        (
            envspec("docker", "", "type: traefik\n  global: true"),
            "gateway global cannot be set for standalone Docker environment",
        ),
        (
            envspec("k8s", "", "placement:\n    constraints: [\"node.role == worker\"]"),
            "gateway placement cannot be set for K8S environment",
        ),
    ];
    for (env, expected) in cases {
        let stderr = prepare_fails(&env);
        assert!(stderr.contains(expected), "expected '{}' in: {}", expected, stderr);
    }
}

/// The fixture env spec with a deployment-wide update policy and placement, and
/// an `api` override of the update policy only.
fn envspec_with_rollout(env_type: &str, extra_top_level: &str, gateway_type: &str) -> String {
    envspec(env_type, extra_top_level, gateway_type).replace(
        "    services:\n      api:\n",
        r#"    defaults:
      update:
        order: stop-first
        parallelism: 2
      placement:
        constraints:
          - node.role == worker
        preferences:
          - spread: node.labels.az
    services:
      api:
        update:
          order: start-first
          failure_action: rollback
          parallelism: 1
          delay: 10s
          monitor: 1m30s
"#,
    )
}

#[test]
fn swarm_services_take_their_update_policy_and_placement() {
    let dir = prepare(&envspec_with_rollout("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let stack = read(&out.join("prod").join("docker-compose.yaml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();

    let api = &parsed["services"]["api"]["deploy"];
    let update = &api["update_config"];
    assert_eq!(update["order"].as_str(), Some("start-first"), "{}", stack);
    assert_eq!(update["failure_action"].as_str(), Some("rollback"), "{}", stack);
    assert_eq!(update["parallelism"].as_u64(), Some(1), "{}", stack);
    assert_eq!(update["delay"].as_str(), Some("10s"), "{}", stack);
    assert_eq!(update["monitor"].as_str(), Some("1m30s"), "{}", stack);
    let rollback = &api["rollback_config"];
    assert_eq!(rollback["order"].as_str(), Some("start-first"), "{}", stack);
    assert_eq!(rollback["parallelism"].as_u64(), Some(1), "{}", stack);
    assert!(rollback["delay"].is_null(), "{}", stack);
    // api sets no placement of its own, so it keeps the deployment's.
    assert_eq!(
        api["placement"]["constraints"][0].as_str(),
        Some("node.role == worker"),
        "{}",
        stack
    );
    assert_eq!(
        api["placement"]["preferences"][0]["spread"].as_str(),
        Some("node.labels.az"),
        "{}",
        stack
    );

    // A service without an override takes the defaults whole; no rollback
    // config without failure_action: rollback.
    let db = &parsed["services"]["primary-db"]["deploy"];
    assert_eq!(db["update_config"]["order"].as_str(), Some("stop-first"), "{}", stack);
    assert_eq!(db["update_config"]["parallelism"].as_u64(), Some(2), "{}", stack);
    assert!(db["update_config"]["failure_action"].is_null(), "{}", stack);
    assert!(db["rollback_config"].is_null(), "{}", stack);
    assert_eq!(
        db["placement"]["constraints"][0].as_str(),
        Some("node.role == worker"),
        "{}",
        stack
    );

    // A job is placed like the services, and never updated.
    let args = job_args(&out, "migrate");
    assert!(has_pair(&args, "--constraint", "node.role == worker"), "{:?}", args);
    assert!(
        has_pair(&args, "--placement-pref", "spread=node.labels.az"),
        "{:?}",
        args
    );
    assert!(!args.iter().any(|a| a.starts_with("--update-")), "{:?}", args);
}

#[test]
fn without_update_or_placement_the_swarm_stack_has_neither() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let stack = read(&out.join("prod").join("docker-compose.yaml"));
    for key in ["update_config", "rollback_config", "placement"] {
        assert!(!stack.contains(key), "{}", stack);
    }
    let args = job_args(&out, "migrate");
    assert!(
        !args.iter().any(|a| a == "--constraint" || a == "--placement-pref"),
        "{:?}",
        args
    );
    // The gateway service; the controller next to it is always placed on a manager.
    let ingress: serde_yaml::Value =
        serde_yaml::from_str(&read(&out.join("ingress").join("docker-compose.yaml"))).unwrap();
    let gateway = &ingress["services"]["traefik"]["deploy"];
    assert!(
        gateway["mode"].is_null() && gateway["placement"].is_null(),
        "{:?}",
        gateway
    );
}

#[test]
fn update_and_placement_mistakes_fail_validation() {
    let swarm = envspec_with_rollout("docker", "swarm_mode: true", "");
    let cases = [
        (
            swarm.replace("order: start-first", "order: blue-green"),
            "Service 'api': invalid update.order 'blue-green'",
        ),
        (
            swarm.replace("failure_action: rollback", "failure_action: retry"),
            "Service 'api': invalid update.failure_action 'retry'",
        ),
        (
            swarm.replace("delay: 10s", "delay: \"10\""),
            "Service 'api': invalid update.delay '10'",
        ),
        (
            swarm.replace("monitor: 1m30s", "monitor: soon"),
            "Service 'api': invalid update.monitor 'soon'",
        ),
        (
            swarm.replace("- node.role == worker", "- node.role"),
            "Deployment 'prod' defaults: invalid placement constraint 'node.role'",
        ),
        (
            envspec_with_rollout("docker", "", ""),
            "update in deployment 'prod' defaults cannot be set for standalone Docker environment",
        ),
        (
            envspec_with_rollout("k8s", "", ""),
            "update in deployment 'prod' defaults cannot be set for K8S environment",
        ),
    ];
    for (env, expected) in cases {
        let stderr = prepare_fails(&env);
        assert!(stderr.contains(expected), "expected '{}' in: {}", expected, stderr);
    }
}
