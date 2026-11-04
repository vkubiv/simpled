//! simpled 2.0.0 Swarm storage: `multi_node` and per-volume bindings. On Swarm a
//! named volume says where it lives, a service follows its volumes, and with
//! `multi_node: false` the whole environment stays on the node running deploy.sh.

mod common;

use common::{envspec, read, simpled, write_project, APPSPEC};
use std::fs;
use std::path::Path;

/// The fixture app with two volumes: the database keeps `pg-data`, and both the
/// api and the migration job use `uploads`.
fn appspec() -> String {
    APPSPEC
        .replace(
            "    configs:\n      - data: /data\n",
            "    configs:\n      - data: /data\n    volumes:\n      - uploads:/uploads\n",
        )
        .replace(
            "    environment:\n      - LOG_LEVEL\n",
            "    environment:\n      - LOG_LEVEL\n    volumes:\n      - uploads:/uploads\n",
        )
        .replace(
            "    image: postgres:16\n",
            "    image: postgres:16\n    volumes:\n      - pg-data:/var/lib/postgresql/data\n",
        )
        + "\nvolumes:\n  - pg-data\n  - uploads\n"
}

/// The swarm fixture env spec with `bindings` as the deployment's `volumes:`
/// block (already indented under the deployment) and `top` at the top level.
fn swarm_env(top: &str, bindings: &str) -> String {
    envspec("docker", &format!("swarm_mode: true\n{top}"), "").replace(
        "    services:\n      api:\n",
        &format!("{bindings}    services:\n      api:\n"),
    )
}

const BOUND: &str = "    volumes:\n      pg-data:\n        node: db-1\n      uploads:\n        nfs:\n          address: fs.example.com\n          path: /shop/uploads\n          options: nfsvers=4.1\n";

fn run(env: &str) -> (tempfile::TempDir, std::process::Output) {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), env);
    fs::write(dir.path().join("bundle").join("appspec.yaml"), appspec()).unwrap();
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    (dir, output)
}

fn prepare(env: &str) -> (tempfile::TempDir, String) {
    let (dir, output) = run(env);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "prepare-deployment failed:\n{}", stderr);
    (dir, stderr)
}

fn fails(env: &str) -> String {
    let (_, output) = run(env);
    assert!(!output.status.success(), "expected a failure");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stack(dir: &Path) -> serde_yaml::Value {
    serde_yaml::from_str(&read(
        &dir.join("docker-deploy").join("prod").join("docker-compose.yaml"),
    ))
    .unwrap()
}

fn constraints(service: &serde_yaml::Value) -> Vec<String> {
    service["deploy"]["placement"]["constraints"]
        .as_sequence()
        .map(|c| c.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn an_unbound_volume_fails_with_a_block_to_paste() {
    let stderr = fails(&swarm_env("", ""));
    assert!(
        stderr.contains("does not say where these volumes live: pg-data, uploads"),
        "{}",
        stderr
    );
    // anyhow indents every line of the message under "Caused by:".
    let lines: Vec<&str> = stderr.lines().map(|l| l.strip_prefix("    ").unwrap_or(l)).collect();
    let snippet = [
        "deployments:",
        "  prod:",
        "    volumes:",
        "      pg-data:",
        "        node: <node hostname>",
    ];
    assert!(lines.windows(snippet.len()).any(|w| w == snippet), "{}", stderr);
    assert!(stderr.contains("set multi_node: false"), "{}", stderr);
}

#[test]
fn bound_volumes_become_stack_volumes_and_pin_their_services() {
    let (dir, _) = prepare(&swarm_env("", BOUND));
    let stack = stack(dir.path());

    let db = &stack["services"]["primary-db"];
    assert_eq!(db["volumes"][0].as_str(), Some("pg-data:/var/lib/postgresql/data"));
    assert_eq!(constraints(db), vec!["node.hostname == db-1"]);

    // NFS is reachable from every node, so the api stays free to move.
    let api = &stack["services"]["api"];
    assert!(api["volumes"]
        .as_sequence()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("uploads:/uploads")));
    assert!(constraints(api).is_empty(), "{:?}", constraints(api));

    assert!(stack["volumes"]["pg-data"].as_mapping().unwrap().is_empty());
    let uploads = &stack["volumes"]["uploads"];
    assert_eq!(uploads["driver"].as_str(), Some("local"));
    assert_eq!(uploads["driver_opts"]["type"].as_str(), Some("nfs"));
    assert_eq!(
        uploads["driver_opts"]["o"].as_str(),
        Some("addr=fs.example.com,nfsvers=4.1")
    );
    assert_eq!(uploads["driver_opts"]["device"].as_str(), Some(":/shop/uploads"));

    let out = dir.path().join("docker-deploy");
    let script = read(&out.join("deploy.sh"));
    // The job mounts the volume the stack created, with the same NFS options.
    assert!(
        script.contains(
            "--mount 'type=volume,source=prod_uploads,target=/uploads,volume-driver=local,\
             volume-opt=device=:/shop/uploads,\"volume-opt=o=addr=fs.example.com,nfsvers=4.1\",volume-opt=type=nfs'"
        ),
        "{}",
        script
    );
    // Nothing bound lives under the deployment directory any more.
    assert!(!script.contains("/volumes/"), "{}", script);
    assert!(!out.join("prod").join("volumes").exists());
    assert!(!script.contains("SIMPLED_NODE_ID"), "{}", script);
}

#[test]
fn host_path_bindings_are_bind_mounts() {
    let bindings = "    volumes:\n      pg-data:\n        node: db-1\n        path: /srv/shop/pg\n      uploads:\n        path: /mnt/shared/uploads\n";
    let (dir, _) = prepare(&swarm_env("", bindings));
    let stack = stack(dir.path());

    let db = &stack["services"]["primary-db"];
    assert_eq!(db["volumes"][0].as_str(), Some("/srv/shop/pg:/var/lib/postgresql/data"));
    assert_eq!(constraints(db), vec!["node.hostname == db-1"]);
    assert!(stack["volumes"].is_null(), "bind mounts declare no stack volume");

    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    // A shared path is created from here; a directory on db-1 cannot be.
    assert!(
        script.contains("[ -e \"/mnt/shared/uploads\" ] || mkdir -p \"/mnt/shared/uploads\""),
        "{}",
        script
    );
    assert!(!script.contains("/srv/shop/pg\" ]"), "{}", script);
}

#[test]
fn a_database_on_shared_storage_and_replicas_on_a_pinned_volume_warn() {
    let bindings =
        "    volumes:\n      pg-data:\n        nfs: fs.example.com:/shop/pg\n      uploads:\n        node: app-1\n";
    let (_, stderr) = prepare(&swarm_env("", bindings));
    assert!(
        stderr.contains("Warning: service 'primary-db' (postgres) keeps volume 'pg-data' on shared storage"),
        "{}",
        stderr
    );
    // api runs 2 replicas in the fixture, and both now land on app-1.
    assert!(
        stderr.contains("Warning: service 'api' runs 2 replicas on volume 'uploads'"),
        "{}",
        stderr
    );
}

#[test]
fn volume_rules_that_cannot_work_fail_validation() {
    let cases = [
        (
            swarm_env(
                "",
                &BOUND.replace("    volumes:\n", "    volumes:\n      cache:\n        node: db-1\n"),
            ),
            "binds volume 'cache', but the app does not declare it. Declared volumes: pg-data, uploads",
        ),
        (
            swarm_env("", BOUND).replace(
                "        prefix: /\n",
                "        prefix: /\n        volumes:\n          - ./data:/seed\n",
            ),
            "Service 'api' mounts the relative path './data' at /seed",
        ),
        (
            swarm_env(
                "",
                "    volumes:\n      pg-data:\n        node: db-1\n      uploads:\n        node: app-1\n",
            )
            .replace(
                "        prefix: /\n",
                "        prefix: /\n        volumes:\n          - pg-data:/pg\n",
            ),
            "Service 'api' mounts volume 'uploads' on node app-1 and volume 'pg-data' on node db-1",
        ),
        (
            swarm_env(
                "",
                "    volumes:\n      pg-data:\n        node: db-1\n      uploads:\n        node: app-1\n",
            )
            .replace(
                "        prefix: /\n",
                "        prefix: /\n        update:\n          order: start-first\n",
            ),
            "Service 'api' mounts volume 'uploads', which lives on node app-1, and updates start-first",
        ),
        (
            swarm_env("", "    volumes:\n      pg-data:\n        path: data/pg\n"),
            "Volume 'pg-data' in deployment 'prod': path 'data/pg' must be absolute",
        ),
        (
            swarm_env("", "    volumes:\n      pg-data:\n        nfs: fs.example.com\n"),
            "nfs 'fs.example.com' must be written as <server>:<export path>",
        ),
        (
            swarm_env(
                "",
                "    volumes:\n      pg-data:\n        node: db-1\n        nfs: fs:/x\n",
            ),
            "nfs cannot be combined with node or path",
        ),
        (
            swarm_env("", "    volumes:\n      pg-data: {}\n"),
            "Volume 'pg-data' in deployment 'prod': set one of node, path or nfs",
        ),
        (
            envspec("k8s", "", "").replace(
                "    services:\n      api:\n",
                "    volumes:\n      pg-data:\n        node: db-1\n    services:\n      api:\n",
            ),
            "volume binding 'pg-data' in deployment 'prod' cannot be set for K8S environment",
        ),
        (
            envspec("k8s", "multi_node: true", ""),
            "multi_node cannot be set for K8S environment",
        ),
        (
            envspec("docker", "multi_node: false", ""),
            "multi_node cannot be set for standalone Docker environment",
        ),
        (
            envspec("docker", "swarm_mode: true\nmulti_node: false", "global: true")
                .replace("    disable: true", "    upstream: true"),
            "gateway global cannot be combined with multi_node: false",
        ),
    ];
    for (env, expected) in cases {
        let stderr = fails(&env);
        assert!(stderr.contains(expected), "expected '{}' in: {}", expected, stderr);
    }
}

#[test]
fn single_node_keeps_the_1x_layout_and_pins_everything_to_the_deploying_node() {
    let (dir, _) = prepare(&swarm_env("multi_node: false\n", ""));
    let out = dir.path().join("docker-deploy");
    let stack = stack(dir.path());

    // Unbound volumes are the 1.x directories next to the stack file.
    let db = &stack["services"]["primary-db"];
    assert_eq!(
        db["volumes"][0].as_str(),
        Some("./volumes/pg-data:/var/lib/postgresql/data")
    );
    assert!(out.join("prod").join("volumes").join("pg-data").is_dir());

    for service in ["api", "primary-db"] {
        assert_eq!(
            constraints(&stack["services"][service]),
            vec!["node.id == ${SIMPLED_NODE_ID}"],
            "{}",
            service
        );
    }
    let ingress: serde_yaml::Value =
        serde_yaml::from_str(&read(&out.join("ingress").join("docker-compose.yaml"))).unwrap();
    assert_eq!(
        constraints(&ingress["services"]["traefik"]),
        vec!["node.id == ${SIMPLED_NODE_ID}"]
    );

    let script = read(&out.join("deploy.sh"));
    let check = script
        .find("awk '$3 == \"Ready\" && $4 == \"Active\"")
        .expect("node check");
    let export = script.find("export SIMPLED_NODE_ID").expect("node id export");
    let first_deploy = script.find("docker stack deploy").unwrap();
    assert!(check < export && export < first_deploy, "{}", script);
    assert!(
        script.contains("but the env spec sets multi_node: false."),
        "{}",
        script
    );
    // The job is pinned too, with the variable left for the shell to expand.
    assert!(
        script.contains("  --constraint \"node.id == ${SIMPLED_NODE_ID}\" \\"),
        "{}",
        script
    );
}
