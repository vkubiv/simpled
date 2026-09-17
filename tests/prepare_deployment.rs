//! End-to-end checks of `simpled prepare-deployment`: a small application and
//! environment are written to a temporary directory, the real binary is run on
//! them, and the generated deployment is inspected. This is what catches a
//! generator that silently drops a spec field or writes an object the target
//! would reject.

use std::fs;
use std::path::Path;
use std::process::Command;

const APPSPEC: &str = r#"
name: shop
version: 1.2.3

environment:
  external:
    - LOG_LEVEL=info
    - PUBLIC_URL

secrets:
  - db_password

configs:
  data:
    - settings.json

app_services:
  api:
    type: public
    image: myorg/api
    environment:
      - $all
    secrets:
      - db_password:
          variable: DB_PASSWORD
    configs:
      - data: /data
    ports:
      - "80:8080"
    depends_on:
      - primary-db
  migrate:
    type: job
    image: myorg/migrate
    environment:
      - LOG_LEVEL
    secrets:
      - db_password:
          variable: DB_PASSWORD
    depends_on:
      - primary-db

extra_services:
  primary-db:
    type: internal
    image: postgres:16
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD", "pg_isready"]
      interval: 5s
"#;

fn envspec(env_type: &str, extra_top_level: &str, gateway_type: &str) -> String {
    format!(
        r#"
type: {env_type}
{extra_top_level}
gateway:
  hosts:
    web: shop.example.com
  tls:
    disable: true
  {gateway_type}

registry:
  myorg: registry.example.com

deployments:
  prod:
    primary_host: web
    application:
      name: shop
    environment:
      - PUBLIC_URL=https://shop.example.com
    secrets:
      db_password:
        env: SHOP_DB_PASSWORD
    configs:
      data: ./data
    services:
      api:
        host: web
        prefix: /
        replicas: 2
"#
    )
}

/// Writes the fixture project and runs `prepare-deployment prod` in it.
fn prepare(env: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("envspec.yaml"), env).unwrap();
    fs::create_dir_all(root.join("bundle")).unwrap();
    fs::write(root.join("bundle").join("appspec.yaml"), APPSPEC).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("data").join("settings.json"), "{}").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_simpled"))
        .current_dir(root)
        .env("SHOP_DB_PASSWORD", "s3cr3t")
        .args(["prepare-deployment", "prod", "--bundle", "bundle"])
        .output()
        .expect("simpled runs");
    assert!(
        output.status.success(),
        "prepare-deployment failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
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
