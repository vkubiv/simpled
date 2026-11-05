//! `gateway.daemon`: the daemon's private API is routed under the given hosts,
//! and its tokens reach the daemon as Swarm secrets (or files, standalone).

mod common;

use common::{envspec, prepare, read, simpled, write_project};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";

fn with_daemon(env_type: &str, extra: &str, daemon: &str) -> String {
    envspec(env_type, extra, &format!("daemon:\n{}", daemon))
}

fn api_on_web(token: &str) -> String {
    format!(
        "    hosts:\n      web:\n        prefix: /_simpled\n    read_token: \"{}\"",
        token
    )
}

fn fails(env: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), env);
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(!output.status.success(), "expected a failure");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn on_swarm_the_token_is_a_gateway_secret_and_the_api_is_routed_under_its_prefix() {
    let dir = prepare(&with_daemon("docker", "swarm_mode: true", &api_on_web(TOKEN)));
    let out = dir.path().join("docker-deploy");

    assert_eq!(read(&out.join("ingress").join("daemon").join("read_token")), TOKEN);
    let text = read(&out.join("ingress").join("docker-compose.yaml"));
    let stack: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let mount = &stack["services"]["daemon"]["secrets"][0];
    assert_eq!(mount["source"].as_str(), Some("gateway_daemon_read_token"), "{}", text);
    assert_eq!(mount["target"].as_str(), Some("/run/secrets/simpled_daemon_read_token"));
    assert!(stack["secrets"]["gateway_daemon_read_token"]["name"]
        .as_str()
        .unwrap()
        .starts_with("${SIMPLED_GATEWAY_OBJECT_"));

    let config: serde_json::Value =
        serde_json::from_str(&read(&out.join("ingress").join("daemon").join("daemon.json"))).unwrap();
    assert_eq!(
        config["api"],
        serde_json::json!({"read_token_file": "/run/secrets/simpled_daemon_read_token"})
    );
    // The token is not in the config, only where to find it.
    assert!(!read(&out.join("ingress").join("daemon").join("daemon.json")).contains(TOKEN));

    let dynamic: serde_yaml::Value =
        serde_yaml::from_str(&read(&out.join("ingress").join("traefik").join("dynamic_conf.yml"))).unwrap();
    let router = &dynamic["http"]["routers"]["simpled-daemon-api-0"];
    assert_eq!(
        router["rule"].as_str(),
        Some("Host(`shop.example.com`) && PathPrefix(`/_simpled/`)")
    );
    assert_eq!(router["service"].as_str(), Some("simpled-daemon"));
    assert_eq!(
        dynamic["http"]["middlewares"]["strip-simpled-daemon-0"]["stripPrefix"]["prefixes"][0].as_str(),
        Some("/_simpled")
    );

    let plan: serde_json::Value = serde_json::from_str(&read(&out.join("ingress").join("plan.json"))).unwrap();
    assert!(plan["objects"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["kind"] == "secret" && o["file"] == "ingress/daemon/read_token"));
}

#[test]
fn an_aws_token_is_fetched_on_the_deploy_target() {
    let daemon = "    hosts:\n      web:\n        prefix: /_simpled\n    read_token:\n      aws: prod/simpled/tokens\n      jq: .read";
    let dir = prepare(&with_daemon("docker", "swarm_mode: true", daemon));
    let out = dir.path().join("docker-deploy");
    assert!(!out.join("ingress").join("daemon").join("read_token").exists());
    let fetch = read(&out.join("fetch-secrets.sh"));
    assert!(fetch.contains("'prod/simpled/tokens'"), "{}", fetch);
    assert!(fetch.contains("ingress/daemon/read_token"), "{}", fetch);
}

#[test]
fn standalone_mounts_the_token_file_into_the_daemon() {
    let dir = prepare(&with_daemon("docker", "", &api_on_web(TOKEN)));
    let out = dir.path().join("docker-deploy");
    assert_eq!(read(&out.join("daemon").join("read_token")), TOKEN);
    let script = read(&out.join("deploy.sh"));
    assert!(
        script.contains("-v $(pwd)/daemon/read_token:/run/secrets/simpled_daemon_read_token:ro"),
        "{}",
        script
    );
}

#[test]
fn daemon_mistakes_fail_validation() {
    let cases = [
        (
            with_daemon("docker", "swarm_mode: true", &api_on_web("short")),
            "gateway.daemon.read_token is 5 characters long; a token needs at least 32",
        ),
        (
            with_daemon(
                "docker",
                "swarm_mode: true",
                &format!(
                    "    hosts:\n      ops:\n        prefix: /\n    read_token: \"{}\"",
                    TOKEN
                ),
            ),
            "gateway.daemon.hosts names 'ops', which is not a host under gateway.hosts",
        ),
        (
            // The app's `api` already has `/` on this host.
            with_daemon(
                "docker",
                "swarm_mode: true",
                &format!(
                    "    hosts:\n      web:\n        prefix: /\n    read_token: \"{}\"",
                    TOKEN
                ),
            ),
            "routes shop.example.com/ to the daemon, but service 'api' of deployment 'prod' is routed there too",
        ),
        (
            with_daemon("k8s", "", &api_on_web(TOKEN)),
            "gateway.daemon is only supported on Docker",
        ),
    ];
    for (env, expected) in cases {
        let stderr = fails(&env);
        assert!(stderr.contains(expected), "expected {:?} in:\n{}", expected, stderr);
    }
}

#[test]
fn without_a_daemon_block_there_is_no_api() {
    let dir = prepare(&envspec("docker", "swarm_mode: true", ""));
    let out = dir.path().join("docker-deploy");
    let config = read(&out.join("ingress").join("daemon").join("daemon.json"));
    assert!(!config.contains("\"api\""), "{}", config);
    assert!(!read(&out.join("ingress").join("docker-compose.yaml")).contains("simpled_daemon_read_token"));
}
