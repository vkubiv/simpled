//! End-to-end checks of `simpled prepare-deployment`: a small application and
//! environment are written to a temporary directory, the real binary is run on
//! them, and the generated deployment is inspected. This is what catches a
//! generator that silently drops a spec field or writes an object the target
//! would reject.

mod common;

use common::{envspec, prepare, read};
use std::fs;

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
    let dir = prepare(&envspec("docker", "swarm_mode: true", "type: nginx"));
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
    assert!(script.contains("run_job 'prod_migrate'"), "{}", script);
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
    let dir = prepare(&envspec_with_resources("docker", "swarm_mode: true", "type: nginx"));
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

    let script = read(&out.join("deploy.sh"));
    assert!(script.contains("  --limit-memory '512m' \\\n"), "{}", script);
    assert!(!script.contains("--limit-cpu"), "{}", script);
    assert!(!script.contains("--reserve-"), "{}", script);
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
    let swarm = prepare(&envspec("docker", "swarm_mode: true", "type: nginx"));
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
    let env = envspec_with_resources("docker", "swarm_mode: true", "type: nginx").replace("memory: 1Gi", "memory: 1GB");
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
    let dir = prepare(&envspec("docker", "swarm_mode: true", "type: nginx"));
    let stack = read(
        &dir.path()
            .join("docker-deploy")
            .join("ingress")
            .join("docker-compose.yaml"),
    );
    assert_eq!(host_mode_ports(&stack, "nginx"), vec![80]);
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
fn trusted_proxies_reach_the_standalone_gateways() {
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

    let dir = prepare(&envspec(
        "docker",
        "",
        "type: nginx\n  trusted_proxies: [\"10.0.0.0/8\"]",
    ));
    let nginx = read(&dir.path().join("docker-deploy").join("nginx").join("default.conf"));
    assert!(
        nginx.starts_with("set_real_ip_from 10.0.0.0/8;\nreal_ip_header X-Forwarded-For;\nreal_ip_recursive on;\n"),
        "{}",
        nginx
    );
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
