//! `/.well-known/simpled/version`: every deployment ships a tiny service that
//! answers with its own version, and each gateway target routes the path on
//! every host to the deployment that owns the host.

mod common;

use common::{envspec, prepare, read};

const PATH: &str = "/.well-known/simpled/version";

fn version_document(conf: &str) -> serde_json::Value {
    let start = conf.find("return 200 '").expect("return 200") + "return 200 '".len();
    let end = conf[start..].find("'; }").expect("closing quote") + start;
    serde_json::from_str(&conf[start..end]).unwrap_or_else(|e| panic!("{}: {}", e, conf))
}

#[test]
fn a_swarm_deployment_ships_its_version_service_and_nginx_routes_to_it() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", "type: nginx"));
    let out = dir.path().join("docker-deploy");

    let stack = read(&out.join("prod").join("docker-compose.yaml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
    let service = &parsed["services"]["simpled-version-prod"];
    assert!(service["image"].as_str().unwrap().starts_with("nginx:"), "{}", stack);
    // Port 80 on the host is the gateway's; the version service is only exposed to it.
    assert!(service["ports"].is_null(), "{}", stack);
    assert_eq!(service["expose"][0].as_str(), Some("80"), "{}", stack);

    let env = read(&out.join("prod").join("simpled-version-prod").join(".env"));
    let conf = env
        .lines()
        .find_map(|l| l.strip_prefix("SIMPLED_VERSION_CONF="))
        .unwrap_or_else(|| panic!("{}", env));
    let doc = version_document(conf);
    assert_eq!(doc["application"], "shop");
    assert_eq!(doc["version"], "1.2.3");
    assert_eq!(doc["deployment"], "prod");
    assert!(doc["deployed_at"].as_u64().unwrap() > 0);

    // Resolved per request, so a gateway naming a deployment that has not been
    // redeployed yet still starts.
    let nginx = read(&out.join("ingress").join("nginx").join("default.conf"));
    assert!(nginx.contains(&format!("location = {} {{", PATH)), "{}", nginx);
    assert!(nginx.contains("resolver 127.0.0.11"), "{}", nginx);
    assert!(
        nginx.contains("set $simpled_version simpled-version-prod;"),
        "{}",
        nginx
    );
}

#[test]
fn traefik_routes_the_path_to_the_deployments_version_service() {
    let dir = prepare(&envspec("docker", "", "type: traefik"));
    let out = dir.path().join("docker-deploy");

    let traefik = read(&out.join("traefik").join("dynamic_conf.yml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&traefik).unwrap();
    let router = &parsed["http"]["routers"]["simpled-version-shop-example-com"];
    assert_eq!(
        router["rule"].as_str(),
        Some(format!("Host(`shop.example.com`) && Path(`{}`)", PATH).as_str()),
        "{}",
        traefik
    );
    assert_eq!(
        parsed["http"]["services"]["simpled-version-shop-example-com"]["loadBalancer"]["servers"][0]["url"].as_str(),
        Some("http://prod_simpled-version-prod:80/"),
        "{}",
        traefik
    );

    let script = read(&out.join("deploy.sh"));
    assert!(script.contains("--name simpled-version-prod"), "{}", script);
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
