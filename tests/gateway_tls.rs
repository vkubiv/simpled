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

    // The daemon answers the HTTP-01 challenges, over plain HTTP: the redirect
    // to HTTPS sits below every router on `web`.
    let challenge = &dynamic["http"]["routers"]["simpled-acme-challenge"];
    assert_eq!(
        challenge["rule"].as_str(),
        Some("PathPrefix(`/.well-known/acme-challenge/`)")
    );
    assert_eq!(challenge["entryPoints"][0].as_str(), Some("web"));
    assert_eq!(challenge["service"].as_str(), Some("simpled-daemon"));
    let traefik: serde_yaml::Value = serde_yaml::from_str(&traefik).unwrap();
    assert_eq!(
        traefik["entryPoints"]["web"]["http"]["redirections"]["entryPoint"]["priority"].as_u64(),
        Some(1)
    );
    let daemon: serde_json::Value =
        serde_json::from_str(&read(&out.join("ingress").join("daemon").join("daemon.json"))).unwrap();
    assert_eq!(
        daemon["acme"],
        serde_json::json!({
            "directory": "https://acme-v02.api.letsencrypt.org/directory",
            "email": "ops@example.com",
            "domains": ["shop.example.com"],
        })
    );

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
    // A 2.0.x acme.json next to deploy.sh is imported first, once.
    let import = script
        .find(
            "if [ -f letsencrypt/acme.json ]; then
  ./simpled step tls-import ingress/plan.json letsencrypt/acme.json
fi",
        )
        .expect(&script);
    assert!(import < step, "{}", script);
}

#[test]
fn standalone_keeps_traefiks_own_acme_client() {
    let dir = prepare(&letsencrypt(&envspec("docker", "", "")));
    let out = dir.path().join("docker-deploy");
    let traefik = read(&out.join("traefik").join("traefik.yml"));
    assert!(traefik.contains("certificatesResolvers"), "{}", traefik);
    assert!(!traefik.contains("priority"), "{}", traefik);
    assert!(!traefik.contains("caServer"), "{}", traefik);

    // Another ACME directory reaches Traefik too.
    let staging = letsencrypt(&envspec("docker", "", "")).replace(
        "      email: ops@example.com",
        "      email: ops@example.com
      server: https://acme-staging-v02.api.letsencrypt.org/directory",
    );
    let dir = prepare(&staging);
    let traefik = read(&dir.path().join("docker-deploy").join("traefik").join("traefik.yml"));
    assert!(
        traefik.contains("caServer: \"https://acme-staging-v02.api.letsencrypt.org/directory\""),
        "{}",
        traefik
    );
    assert!(!read(&out.join("deploy.sh")).contains("tls-secrets"));
    let daemon = read(&out.join("daemon").join("daemon.json"));
    assert!(!daemon.contains("acme"), "{}", daemon);
}

#[test]
fn without_letsencrypt_there_are_no_certificate_secrets() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let stack = read(&out.join("ingress").join("docker-compose.yaml"));
    assert!(!stack.contains("tls_1"), "{}", stack);
    assert!(!read(&out.join("deploy.sh")).contains("tls-secrets"));
}
