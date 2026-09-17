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
