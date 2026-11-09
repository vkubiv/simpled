//! The labels the simpled controller reads: every long-running service of a
//! deployment carries its deployment, application, version, deploy time and
//! spec name, on Swarm and on standalone Docker.

mod common;

use common::{envspec, prepare, read};

const LABELS: [(&str, &str); 5] = [
    ("simpled.application", "shop"),
    ("simpled.deployed_at", "${SIMPLED_DEPLOYED_AT}"),
    ("simpled.deployment", "prod"),
    ("simpled.service", "api"),
    ("simpled.version", "1.2.3"),
];

#[test]
fn swarm_services_carry_the_labels_as_service_labels() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let stack: serde_yaml::Value = serde_yaml::from_str(&read(&out.join("prod").join("docker-compose.yaml"))).unwrap();

    // deploy.labels: service labels, which change no task, so a deploy that
    // only moves the time restarts nothing.
    let labels = &stack["services"]["api"]["deploy"]["labels"];
    for (key, value) in LABELS {
        assert_eq!(labels[key].as_str(), Some(value), "{:?}", labels);
    }
    assert_eq!(
        stack["services"]["primary-db"]["deploy"]["labels"]["simpled.service"].as_str(),
        Some("primary-db")
    );
    // Not on the containers, and not on the version sidecar.
    assert!(stack["services"]["api"]["labels"].is_null());
    assert!(stack["services"]["simpled-version-prod"]["deploy"]["labels"].is_null());

    // The time is set before any stack deploys.
    let script = read(&out.join("deploy.sh"));
    let set = script
        .find("export SIMPLED_DEPLOYED_AT=\"$(date +%s)\"")
        .expect(&script);
    assert!(set < script.find("docker stack deploy").unwrap(), "{}", script);
    // The job is not a service the controller reports on.
    let plan = read(&out.join("prod").join("plan.json"));
    assert!(!plan.contains("simpled.service"), "{}", plan);
}

#[test]
fn standalone_containers_carry_the_labels() {
    let dir = prepare(&envspec("docker", "", ""));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    let run_line = |name: &str| {
        script
            .lines()
            .find(|l| {
                l.starts_with(&format!("docker run -d --name {} ", name))
                    || l.starts_with(&format!("docker run --name {} ", name))
            })
            .unwrap_or_else(|| panic!("no docker run for {}:\n{}", name, script))
            .to_string()
    };

    let api = run_line("prod_api");
    assert!(api.contains(" --label 'simpled.deployment=prod'"), "{}", api);
    assert!(api.contains(" --label 'simpled.service=api'"), "{}", api);
    assert!(api.contains(" --label 'simpled.version=1.2.3'"), "{}", api);
    assert!(
        api.contains(" --label \"simpled.deployed_at=${SIMPLED_DEPLOYED_AT}\""),
        "{}",
        api
    );
    assert!(!run_line("prod_migrate").contains("--label"));

    let set = script.find("export SIMPLED_DEPLOYED_AT=").expect(&script);
    assert!(set < script.find("docker run").unwrap(), "{}", script);
}

#[test]
fn standalone_removes_the_containers_of_retired_services() {
    let dir = prepare(&envspec("docker", "", ""));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    let start = script
        .find("docker ps -a --filter 'label=simpled.deployment=prod' --filter 'label=simpled.service'")
        .unwrap_or_else(|| panic!("no cleanup of retired services:\n{}", script));
    let cleanup = &script[start..];
    // The labelled services are kept; the job is not labelled, so not listed.
    assert!(cleanup.contains("    'api'|'primary-db') ;;"), "{}", cleanup);
    assert!(cleanup.contains("docker rm -f \"$id\""), "{}", cleanup);
    // After the services are started, so a failed start leaves the old ones.
    assert!(
        script.rfind("docker run -d --name prod_api ").unwrap() < start,
        "{}",
        script
    );
}
