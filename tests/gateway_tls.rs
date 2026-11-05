//! On Docker Swarm the gateway's certificates are Swarm secrets, one per domain,
//! which every node has. Traefik runs no ACME there: it reads the files.

mod common;

use common::{envspec, prepare, read};

fn letsencrypt(env: &str) -> String {
    env.replace("    disable: true", "    letsencrypt:\n      email: ops@example.com")
}

#[test]
fn the_swarm_gateway_mounts_one_certificate_secret_per_domain() {
    let dir = prepare(&letsencrypt(&envspec("docker", "swarm_mode: true", "")));
    let out = dir.path().join("docker-deploy");

    let text = read(&out.join("ingress").join("docker-compose.yaml"));
    let stack: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let mount = &stack["services"]["traefik"]["secrets"][0];
    assert_eq!(mount["source"].as_str(), Some("tls_1"), "{}", text);
    assert_eq!(
        mount["target"].as_str(),
        Some("/run/secrets/simpled_tls/shop.example.com.pem")
    );
    assert_eq!(stack["secrets"]["tls_1"]["name"].as_str(), Some("${SIMPLED_TLS_1}"));
    // No ACME store any more.
    assert!(stack["services"]["traefik"]["volumes"].is_null(), "{}", text);
    assert!(!out.join("letsencrypt").exists());

    let traefik = read(&out.join("ingress").join("traefik").join("traefik.yml"));
    assert!(!traefik.contains("certificatesResolvers"), "{}", traefik);
    let dynamic = read(&out.join("ingress").join("traefik").join("dynamic_conf.yml"));
    let dynamic: serde_yaml::Value = serde_yaml::from_str(&dynamic).unwrap();
    let cert = &dynamic["tls"]["certificates"][0];
    assert_eq!(
        cert["certFile"].as_str(),
        Some("/run/secrets/simpled_tls/shop.example.com.pem")
    );
    assert_eq!(cert["keyFile"], cert["certFile"]);
    for (name, router) in dynamic["http"]["routers"].as_mapping().unwrap() {
        if router["entryPoints"][0].as_str() == Some("websecure") {
            assert!(router["tls"].is_mapping(), "{:?}", name);
            assert!(router["tls"]["certResolver"].is_null(), "{:?}", name);
        }
    }

    let plan: serde_json::Value = serde_json::from_str(&read(&out.join("ingress").join("plan.json"))).unwrap();
    assert_eq!(
        plan["tls"],
        serde_json::json!([{"domain": "shop.example.com", "var": "SIMPLED_TLS_1"}])
    );
    let script = read(&out.join("deploy.sh"));
    let step = script
        .find("tls=\"$(./simpled step tls-secrets ingress/plan.json)\"")
        .expect(&script);
    assert!(
        step < script.find("docker stack deploy -c ingress/").unwrap(),
        "{}",
        script
    );
}

#[test]
fn standalone_keeps_traefiks_own_acme_client() {
    let dir = prepare(&letsencrypt(&envspec("docker", "", "")));
    let out = dir.path().join("docker-deploy");
    let traefik = read(&out.join("traefik").join("traefik.yml"));
    assert!(traefik.contains("certificatesResolvers"), "{}", traefik);
    assert!(!read(&out.join("deploy.sh")).contains("tls-secrets"));
}

#[test]
fn without_letsencrypt_there_are_no_certificate_secrets() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let stack = read(&out.join("ingress").join("docker-compose.yaml"));
    assert!(!stack.contains("tls_1"), "{}", stack);
    assert!(!read(&out.join("deploy.sh")).contains("tls-secrets"));
}
