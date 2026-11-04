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

    // Each owner's plan lists its objects, which the steps sync and prune.
    let plan = |path: &std::path::Path| -> serde_json::Value { serde_json::from_str(&read(path)).unwrap() };
    let gateway = plan(&out.join("ingress").join("plan.json"));
    assert_eq!(gateway["owner_label"], "simpled.scope=environment");
    assert_eq!(gateway["objects"][0]["base"], "gateway_traefik.yml");
    assert_eq!(gateway["objects"][0]["file"], "ingress/traefik/traefik.yml");
    assert_eq!(gateway["objects"][1]["var"], "SIMPLED_GATEWAY_OBJECT_2");
    let deployment = plan(&out.join("prod").join("plan.json"));
    assert_eq!(deployment["owner_label"], "simpled.deployment=prod");
    assert_eq!(deployment["objects"][0]["kind"], "config");
    assert_eq!(deployment["objects"][0]["file"], "prod/configs/shop-data/settings.json");
    assert_eq!(deployment["simpled_version"], env!("CARGO_PKG_VERSION"));

    let script = read(&out.join("deploy.sh"));
    let gateway = script
        .find("gateway_objects=\"$(./simpled step sync-objects ingress/plan.json)\"\neval \"${gateway_objects}\"")
        .expect("gateway files are synced");
    let config = script
        .find("objects=\"$(./simpled step sync-objects prod/plan.json)\"\neval \"${objects}\"")
        .expect("deployment configs are synced");
    let first_deploy = script.find("docker stack deploy").unwrap();
    assert!(gateway < first_deploy && config < first_deploy, "{}", script);

    // Pruning comes after the last deploy.
    let last_deploy = script.rfind("docker stack deploy").unwrap();
    for line in [
        "./simpled step prune-objects ingress/plan.json",
        "./simpled step prune-objects prod/plan.json",
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

    let plan: serde_json::Value =
        serde_json::from_str(&read(&dir.path().join("docker-deploy").join("prod").join("plan.json"))).unwrap();
    let args: Vec<&str> = plan["jobs"][0]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    assert!(
        args.windows(2).any(|w| w
            == [
                "--config",
                "source=${SIMPLED_OBJECT_1},target=/data/settings.json,mode=444"
            ]),
        "{:?}",
        args
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

/// Nothing time-dependent is left in a Swarm service's spec, so a deploy restarts
/// only the services whose image, environment, configs or secrets changed. The
/// gateway still carries `DEPLOY_DATE` and restarts on every deploy.
#[test]
fn preparing_twice_gives_the_same_stack_file() {
    let first = prepare(&swarm());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = prepare(&swarm());
    let stack = |dir: &tempfile::TempDir| {
        read(
            &dir.path()
                .join("docker-deploy")
                .join("prod")
                .join("docker-compose.yaml"),
        )
    };

    assert_eq!(stack(&first), stack(&second));
    assert!(!stack(&first).contains("DEPLOY_DATE"), "{}", stack(&first));
    let ingress = read(
        &first
            .path()
            .join("docker-deploy")
            .join("ingress")
            .join("docker-compose.yaml"),
    );
    assert!(ingress.contains("DEPLOY_DATE="), "{}", ingress);
}

#[test]
fn standalone_docker_keeps_its_bind_mounts() {
    let dir = prepare(&envspec("docker", "", ""));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    assert!(!script.contains("sync_object"), "{}", script);
    assert!(script.contains("-v $(pwd)/configs/shop-data:/data"), "{}", script);
}

#[test]
fn target_binary_flag_wins_over_the_environment() {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), &swarm());
    fs::write(dir.path().join("arm-simpled"), "arm build").unwrap();
    let output = simpled(
        dir.path(),
        &[
            "prepare-deployment",
            "prod",
            "--bundle",
            "bundle",
            "--target-binary",
            "arm-simpled",
        ],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(read(&dir.path().join("docker-deploy").join("simpled")), "arm build");
}

#[test]
fn target_platform_is_validated() {
    let cases = [
        (
            envspec("docker", "swarm_mode: true\ntarget_platform: linux-riscv", ""),
            "Unknown target_platform 'linux-riscv': expected linux-amd64 or linux-arm64",
        ),
        (
            envspec("docker", "target_platform: linux-arm64", ""),
            "target_platform cannot be set for standalone Docker environment",
        ),
        (
            envspec("k8s", "target_platform: linux-arm64", ""),
            "target_platform cannot be set for K8S environment",
        ),
    ];
    for (env, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        write_project(dir.path(), &env);
        let output = simpled(
            dir.path(),
            &["prepare-deployment", "prod", "--bundle", "bundle"],
            &[("SHOP_DB_PASSWORD", "s3cr3t")],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success() && stderr.contains(expected),
            "expected '{}' in: {}",
            expected,
            stderr
        );
    }
}

/// Several deployments of one env spec share the gateway and the swarm: a deploy
/// holds the swarm's lock for its whole run and checks the gateway it replaces.
#[test]
fn a_deploy_holds_the_lock_and_checks_the_gateway_it_replaces() {
    let dir = prepare(&swarm());
    let out = dir.path().join("docker-deploy");
    let script = read(&out.join("deploy.sh"));

    let lock = script
        .find("lock=\"$(./simpled step lock 'prod')\"\neval \"${lock}\"\ntrap './simpled step unlock' EXIT\n")
        .expect("the lock is taken and released on exit");
    let check = script
        .find("./simpled step check-gateway ingress/plan.json\ndocker stack deploy -c ingress/docker-compose.yaml")
        .expect("the gateway is checked right before it is deployed");
    let sync = script.find("./simpled step sync-objects ingress/plan.json").unwrap();
    // Locked before the first change to the swarm; checked once the new
    // gateway objects' names are known.
    assert!(lock < sync && sync < check, "{}", script);

    let ingress: serde_yaml::Value =
        serde_yaml::from_str(&read(&out.join("ingress").join("docker-compose.yaml"))).unwrap();
    let labels = &ingress["services"]["traefik"]["deploy"]["labels"];
    assert_eq!(labels["simpled.version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    assert_eq!(labels["simpled.deployment"].as_str(), Some("prod"));
}
