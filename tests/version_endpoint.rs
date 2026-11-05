//! `/.well-known/simpled/version` and `/health` on every gateway host. On Docker
//! the environment's controller answers them, next to the gateway; on Kubernetes
//! every deployment ships a tiny nginx service that answers with its version.

mod common;

use common::{envspec, prepare, read};

const PATH: &str = "/.well-known/simpled/version";
const HEALTH: &str = "/.well-known/simpled/health";

#[test]
fn the_swarm_gateway_stack_runs_the_controller_and_no_deployment_ships_a_sidecar() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");

    let stack: serde_yaml::Value = serde_yaml::from_str(&read(&out.join("prod").join("docker-compose.yaml"))).unwrap();
    assert!(stack["services"]["simpled-version-prod"].is_null());

    let ingress_text = read(&out.join("gateway").join("docker-compose.yaml"));
    let ingress: serde_yaml::Value = serde_yaml::from_str(&ingress_text).unwrap();
    let controller = &ingress["services"]["controller"];
    assert_eq!(
        controller["image"].as_str(),
        Some(
            format!(
                "${{SIMPLED_CONTROLLER_IMAGE:-ghcr.io/vkubiv/simpled:{}}}",
                env!("CARGO_PKG_VERSION")
            )
            .as_str()
        ),
        "{}",
        ingress_text
    );
    assert_eq!(
        controller["volumes"][0].as_str(),
        Some("/var/run/docker.sock:/var/run/docker.sock")
    );
    assert_eq!(controller["read_only"].as_bool(), Some(true));
    assert_eq!(controller["cap_drop"][0].as_str(), Some("ALL"));
    assert_eq!(
        controller["deploy"]["placement"]["constraints"][0].as_str(),
        Some("node.role == manager")
    );
    assert_eq!(
        controller["configs"][0]["target"].as_str(),
        Some("/etc/simpled/controller.json")
    );
    assert_eq!(
        ingress["configs"]["gateway_controller.json"]["name"].as_str(),
        Some("${SIMPLED_GATEWAY_OBJECT_3}")
    );

    // Only the gateway and the controller are on the control network.
    let networks = |service: &str| -> Vec<String> {
        ingress["services"][service]["networks"]
            .as_mapping()
            .unwrap()
            .keys()
            .map(|k| k.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(networks("controller"), vec!["simpled_control"]);
    assert_eq!(networks("gateway"), vec!["default", "simpled_control"]);
    assert_eq!(
        ingress["networks"]["simpled_control"]["driver"].as_str(),
        Some("overlay")
    );

    let config: serde_json::Value =
        serde_json::from_str(&read(&out.join("gateway").join("controller").join("controller.json"))).unwrap();
    assert_eq!(config["mode"], "swarm");
    assert_eq!(
        config["owners"],
        serde_json::json!([{"domain": "shop.example.com", "deployment": "prod"}])
    );
}

#[test]
fn traefik_routes_version_and_health_to_the_controller() {
    let dir = prepare(&envspec("docker", "", "type: traefik"));
    let out = dir.path().join("docker-deploy");

    let traefik = read(&out.join("traefik").join("dynamic_conf.yml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&traefik).unwrap();
    let router = &parsed["http"]["routers"]["simpled-version-shop-example-com"];
    assert_eq!(
        router["rule"].as_str(),
        Some(format!("Host(`shop.example.com`) && (Path(`{}`) || Path(`{}`))", PATH, HEALTH).as_str()),
        "{}",
        traefik
    );
    assert_eq!(router["service"].as_str(), Some("simpled-controller"));
    assert_eq!(
        parsed["http"]["services"]["simpled-controller"]["loadBalancer"]["servers"][0]["url"].as_str(),
        Some("http://simpled_controller:8080/"),
        "{}",
        traefik
    );

    // Standalone: the controller's container, as non-root with the socket's group,
    // and the gateway joins its network.
    let script = read(&out.join("deploy.sh"));
    assert!(!script.contains("simpled-version-prod"), "{}", script);
    let controller = script
        .find("docker run -d --name simpled-controller --network simpled_control --network-alias simpled_controller")
        .expect(&script);
    assert!(
        script.contains("--group-add \"$(stat -c %g /var/run/docker.sock)\""),
        "{}",
        script
    );
    assert!(
        controller < script.find("docker run -d --name simpled-gateway").unwrap(),
        "{}",
        script
    );
    assert!(
        script.contains("docker network connect simpled_control simpled-gateway"),
        "{}",
        script
    );
    let config: serde_json::Value =
        serde_json::from_str(&read(&out.join("controller").join("controller.json"))).unwrap();
    assert_eq!(config["mode"], "standalone");
}

#[test]
fn kubernetes_gets_a_version_deployment_and_an_ingress_of_its_own() {
    let dir = prepare(&envspec("k8s", "", ""));
    let out = dir.path().join("manifests");

    let deployment = read(&out.join("deployment-simpled-version-prod.yaml"));
    assert!(deployment.contains("image: nginx:"), "{}", deployment);
    assert!(out.join("service-simpled-version-prod.yaml").exists());

    let ingress = read(&out.join("ingress.yaml"));
    let docs: Vec<serde_yaml::Value> = ingress
        .split("\n---\n")
        .map(|d| serde_yaml::from_str(d).unwrap())
        .collect();
    let version = docs
        .iter()
        .find(|d| d["metadata"]["name"].as_str() == Some("gateway--simpled-version"))
        .unwrap_or_else(|| panic!("{}", ingress));
    // No annotations: a rewrite-target on the main Ingress must not reach this path.
    assert!(version["metadata"]["annotations"].is_null(), "{}", ingress);
    let rule = &version["spec"]["rules"][0];
    assert_eq!(rule["host"].as_str(), Some("shop.example.com"));
    assert_eq!(rule["http"]["paths"][0]["path"].as_str(), Some(PATH));
    assert_eq!(rule["http"]["paths"][0]["pathType"].as_str(), Some("Exact"));
    assert_eq!(
        rule["http"]["paths"][0]["backend"]["service"]["name"].as_str(),
        Some("simpled-version-prod")
    );
}
