//! simpled 2.0.0 on Swarm: configs, file secrets and the gateway's files are
//! content-addressed Swarm objects, which reach every node through Raft, instead of
//! bind mounts of the deploy directory, which exists on one node only.

mod common;

use common::{envspec, prepare, read, simpled, write_project};
use std::fs;

fn swarm() -> String {
    envspec("docker", "swarm_mode: true", "")
}

#[test]
fn configs_become_swarm_configs_synced_before_the_stacks_deploy() {
    let dir = prepare(&swarm());
    let out = dir.path().join("docker-deploy");

    let stack: serde_yaml::Value = serde_yaml::from_str(&read(&out.join("prod").join("docker-compose.yaml"))).unwrap();
    // `data` is mounted as a directory, so its one file lands inside it.
    let mount = &stack["services"]["api"]["configs"][0];
    assert_eq!(mount["source"].as_str(), Some("prod_shop-data_settings.json"));
    assert_eq!(mount["target"].as_str(), Some("/data/settings.json"));
    assert_eq!(
        stack["configs"]["prod_shop-data_settings.json"]["name"].as_str(),
        Some("${SIMPLED_OBJECT_1}")
    );
    // Nothing is bind-mounted from the deploy directory any more.
    assert!(stack["services"]["api"]["volumes"].is_null());
    assert_eq!(
        read(&out.join("prod").join("configs").join("shop-data").join("settings.json")),
        "{}"
    );

    let ingress = read(&out.join("ingress").join("docker-compose.yaml"));
    let ingress: serde_yaml::Value = serde_yaml::from_str(&ingress).unwrap();
    let traefik = &ingress["services"]["traefik"];
    assert!(traefik["volumes"].is_null(), "{:?}", traefik);
    let targets: Vec<&str> = traefik["configs"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|c| c["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        vec!["/etc/traefik/traefik.yml", "/etc/traefik/dynamic_conf.yml"]
    );
    assert_eq!(
        ingress["configs"]["gateway_dynamic_conf.yml"]["name"].as_str(),
        Some("${SIMPLED_GATEWAY_OBJECT_2}")
    );

    let script = read(&out.join("deploy.sh"));
    let gateway = script
        .find("SIMPLED_GATEWAY_OBJECT_1=\"$(sync_object config 'simpled.scope=environment' 'gateway_traefik.yml' 'ingress/traefik/traefik.yml')\"")
        .expect("gateway files are synced");
    let config = script
        .find("SIMPLED_OBJECT_1=\"$(sync_object config 'simpled.deployment=prod' 'prod_shop-data_settings.json' 'prod/configs/shop-data/settings.json')\"")
        .expect("deployment configs are synced");
    let first_deploy = script.find("docker stack deploy").unwrap();
    assert!(gateway < first_deploy && config < first_deploy, "{}", script);

    // Pruning is scoped to each owner and comes after the last deploy.
    let last_deploy = script.rfind("docker stack deploy").unwrap();
    for line in [
        "prune_objects config 'simpled.scope=environment' 'gateway_traefik.yml' 'gateway_dynamic_conf.yml'",
        "prune_objects config 'simpled.deployment=prod' 'prod_shop-data_settings.json'",
    ] {
        assert!(
            script.find(line).is_some_and(|i| i > last_deploy),
            "{}\n{}",
            line,
            script
        );
    }
}

#[test]
fn a_job_mounts_its_configs_through_the_exported_names() {
    let env = swarm();
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), &env);
    // Give the migration job the config the api has.
    let appspec = fs::read_to_string(dir.path().join("bundle").join("appspec.yaml"))
        .unwrap()
        .replace(
            "    environment:\n      - LOG_LEVEL\n",
            "    environment:\n      - LOG_LEVEL\n    configs:\n      - data: /data\n",
        );
    fs::write(dir.path().join("bundle").join("appspec.yaml"), appspec).unwrap();
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    assert!(
        script.contains("  --config \"source=${SIMPLED_OBJECT_1},target=/data/settings.json,mode=444\" \\"),
        "{}",
        script
    );
}

#[test]
fn a_config_file_over_the_swarm_limit_fails() {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), &swarm());
    fs::write(dir.path().join("data").join("settings.json"), vec![b'x'; 600 * 1024]).unwrap();
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("File 'settings.json' of config 'shop-data' is 600 KB, but a Docker Swarm config or secret holds at most 500 KB"),
        "{}",
        stderr
    );
}

#[test]
fn standalone_docker_keeps_its_bind_mounts() {
    let dir = prepare(&envspec("docker", "", ""));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    assert!(!script.contains("sync_object"), "{}", script);
    assert!(script.contains("-v $(pwd)/configs/shop-data:/data"), "{}", script);
}
