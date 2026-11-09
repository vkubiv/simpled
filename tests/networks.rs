//! Networks on Docker: each deployment on its own, joined by the env spec's
//! `networks:` only.

mod common;

use common::{envspec, prepare, read, simpled, write_project};

/// The fixture's `prod` deployment joining `prod_networks`, next to an `infra`
/// deployment of the same app that joins `infra_networks`.
fn env(extra_top_level: &str, prod_networks: &str, infra_networks: &str) -> String {
    let base = envspec("docker", extra_top_level, "");
    format!(
        "{base}    networks: {prod_networks}
  infra:
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
    networks: {infra_networks}
networks:
  data:
"
    )
}

const SWARM: &str = "swarm_mode: true";

fn yaml(text: &str) -> serde_yaml::Value {
    serde_yaml::from_str(text).unwrap()
}

fn keys(value: &serde_yaml::Value) -> Vec<String> {
    value
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect()
}

fn job_args(out: &std::path::Path) -> Vec<String> {
    let plan: serde_json::Value = serde_json::from_str(&read(&out.join("prod").join("plan.json"))).unwrap();
    plan["jobs"][0]["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap().to_string())
        .collect()
}

fn networks_of(args: &[String]) -> Vec<&str> {
    args.windows(2)
        .filter(|w| w[0] == "--network")
        .map(|w| w[1].as_str())
        .collect()
}

fn fails(env: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), env);
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(!output.status.success(), "prepare-deployment succeeded");
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn a_swarm_stack_is_on_its_own_network_and_the_named_services_join_a_shared_one() {
    let dir = prepare(&env(SWARM, "{data: [api]}", "[data]"));
    let out = dir.path().join("docker-deploy");

    let stack = yaml(&read(&out.join("prod").join("docker-compose.yaml")));
    assert_eq!(
        keys(&stack["services"]["api"]["networks"]),
        vec!["default", "simpled_data"]
    );
    assert_eq!(keys(&stack["services"]["primary-db"]["networks"]), vec!["default"]);
    assert_eq!(
        stack["services"]["api"]["networks"]["default"]["aliases"][0].as_str(),
        Some("api")
    );
    assert_eq!(stack["networks"]["default"]["name"].as_str(), Some("simpled-prod"));
    assert_eq!(stack["networks"]["simpled_data"]["name"].as_str(), Some("simpled_data"));
    assert_eq!(stack["networks"]["simpled_data"]["external"].as_bool(), Some(true));

    // The job is not a member: its own network only.
    assert_eq!(networks_of(&job_args(&out)), vec!["simpled-prod"]);

    let gateway = yaml(&read(&out.join("gateway").join("docker-compose.yaml")));
    assert_eq!(
        keys(&gateway["services"]["gateway"]["networks"]),
        vec!["default", "simpled-infra", "simpled-prod", "simpled_control"]
    );
    assert_eq!(gateway["networks"]["default"]["name"].as_str(), Some("common_network"));

    let script = read(&out.join("deploy.sh"));
    for network in ["simpled-infra", "simpled-prod", "simpled_data", "common_network"] {
        assert!(
            script.contains(&format!(
                "docker network inspect {network} >/dev/null 2>&1 || docker network create --driver overlay --attachable {network} >/dev/null"
            )),
            "{network}:\n{script}"
        );
    }
    let created = script.find("docker network create").unwrap();
    assert!(created < script.find("docker stack deploy").unwrap(), "{}", script);
}

#[test]
fn the_list_form_takes_every_service_jobs_included() {
    let dir = prepare(&env(SWARM, "[data]", "[data]"));
    let out = dir.path().join("docker-deploy");
    let stack = yaml(&read(&out.join("prod").join("docker-compose.yaml")));
    assert_eq!(
        keys(&stack["services"]["primary-db"]["networks"]),
        vec!["default", "simpled_data"]
    );
    assert_eq!(networks_of(&job_args(&out)), vec!["simpled-prod", "simpled_data"]);
}

#[test]
fn without_networks_a_deployment_joins_none_but_its_own() {
    let dir = prepare(&envspec("docker", SWARM, ""));
    let out = dir.path().join("docker-deploy");
    let stack = yaml(&read(&out.join("prod").join("docker-compose.yaml")));
    assert_eq!(keys(&stack["networks"]), vec!["default"]);
    assert_eq!(stack["networks"]["default"]["name"].as_str(), Some("simpled-prod"));
}

#[test]
fn a_standalone_member_is_connected_before_it_starts() {
    let dir = prepare(&env("", "{data: [api, migrate]}", "[data]"));
    let script = read(&dir.path().join("docker-deploy").join("deploy.sh"));
    let at = |needle: &str| script.find(needle).unwrap_or_else(|| panic!("no {needle}:\n{script}"));

    assert!(script.contains(
        "docker network inspect simpled_data >/dev/null 2>&1 || docker network create simpled_data >/dev/null"
    ));
    let db = at("docker run -d --name prod_primary-db --network simpled-prod --network-alias primary-db ");

    let job = at("docker create --name prod_migrate --network simpled-prod --network-alias migrate ");
    let job_connect = at("docker network connect simpled_data prod_migrate\n");
    let job_start = at("docker start -a prod_migrate\n");
    assert!(db < job && job < job_connect && job_connect < job_start, "{}", script);

    let api = at("docker create --name prod_api --network simpled-prod --network-alias api ");
    let connect = at("docker network connect simpled_data prod_api\n");
    let start = at("docker start prod_api >/dev/null\n");
    assert!(job_start < api && api < connect && connect < start, "{}", script);

    // The pre-2.4 container held the same host ports, so it goes first.
    let retire = at("retire_legacy_container 'api' 'prod'\n");
    assert!(retire < api, "{}", script);

    assert!(
        script.contains("docker run -d --name simpled-gateway --network common_network "),
        "{}",
        script
    );
    for network in ["simpled-infra", "simpled-prod", "simpled_control"] {
        assert!(
            script.contains(&format!("docker network connect {network} simpled-gateway\n")),
            "{network}:\n{script}"
        );
    }
}

#[test]
fn a_network_must_be_declared() {
    let env = env(SWARM, "[data, cache]", "[data]");
    let err = fails(&env);
    assert!(
        err.contains("Deployment 'prod' joins network 'cache', which the env spec does not declare"),
        "{err}"
    );
}

#[test]
fn a_network_joined_by_one_deployment_connects_nothing() {
    let err = fails(&env(SWARM, "[data]", "[]"));
    assert!(
        err.contains("Network 'data' connects nothing: only deployment 'prod' joins it"),
        "{err}"
    );
}

#[test]
fn a_member_must_be_a_service_of_the_app() {
    let err = fails(&env(SWARM, "{data: [ap1]}", "[data]"));
    assert!(
        err.contains("Network 'data' in deployment 'prod' lists service 'ap1', which application 'shop' does not have"),
        "{err}"
    );
}

#[test]
fn the_control_network_name_is_reserved() {
    let err =
        fails(&env(SWARM, "[data]", "[data]").replace("networks:\n  data:\n", "networks:\n  data:\n  control:\n"));
    assert!(err.contains("Network name 'control' is reserved"), "{err}");
}

#[test]
fn networks_are_docker_only() {
    let k8s = env(SWARM, "[data]", "[data]")
        .replace("type: docker", "type: k8s")
        .replace(SWARM, "")
        .replace("controller:\n  host: web", "");
    let err = fails(&k8s);
    assert!(err.contains("networks cannot be set for K8S environment"), "{err}");
}
