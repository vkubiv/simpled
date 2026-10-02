//! End-to-end check of named ports and instances through `simpled local
//! generate-config`: an instance shifts every `$port(name)`, and gets a compose
//! project, output directory and container names of its own.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const APPSPEC: &str = r#"
name: shop
version: 1.2.3

environment:
  external:
    - PUBLIC_URL
    - DB_URL
    - MACHINE_ID

app_services:
  api:
    type: public
    image: myorg/api
    environment:
      - $all
    ports:
      - "80:8080"
  worker:
    type: internal
    image: myorg/worker
    environment:
      - $all
    ports:
      - "80:8080"
"#;

const INFRA: &str = r#"
extra_services:
  db:
    image: postgres:16
    ports:
      - "$port(db):5432"
"#;

const LOCALENV: &str = r#"
ports:
  web: 8080
  worker: 8081
  db: 5432
port_step: 3000

gateway:
  hosts:
    web: localhost:$port(web)

deployments:
  dev:
    primary_host: web
    application:
      name: shop
      extra:
        - ./infra.yaml
    environment:
      - PUBLIC_URL=http://localhost:$port(web)
      - DB_URL=postgres://db:5432/shop
      - MACHINE_ID=dev-$instance()
    undockerized_environment:
      - DB_URL=postgres://localhost:$port(db)/shop
    services:
      api:
        host: web
        prefix: /
        ports:
          - "$port(web):80"
      worker:
        ports:
          - "$port(worker):80"
        working_dir: ./worker-src
"#;

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

fn project(localenv: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("localenv.yaml"), localenv).unwrap();
    fs::write(dir.path().join("appspec.yaml"), APPSPEC).unwrap();
    fs::write(dir.path().join("infra.yaml"), INFRA).unwrap();
    dir
}

fn generate(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_simpled"));
    command.current_dir(root).args(["local", "generate-config"]).args(args);
    command.env_remove("SIMPLED_INSTANCE");
    for (k, v) in env {
        command.env(k, v);
    }
    command.output().expect("simpled runs")
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "generate-config failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn compose(dir: &Path) -> serde_yaml::Value {
    serde_yaml::from_str(&read(&dir.join("docker-compose.yaml"))).unwrap()
}

#[test]
fn instance_zero_is_the_stack_as_written() {
    let dir = project(LOCALENV);
    let root = dir.path();
    assert_ok(&generate(root, &[], &[]));

    let parsed = compose(&root.join("local_env"));
    assert_eq!(parsed["name"].as_str(), Some("shop_local"));
    assert_eq!(parsed["services"]["api"]["container_name"].as_str(), Some("api"));
    assert_eq!(parsed["services"]["api"]["ports"][0].as_str(), Some("8080:80"));
    assert_eq!(parsed["services"]["db"]["ports"][0].as_str(), Some("5432:5432"));

    let api_env = read(&root.join("local_env").join("api").join(".env"));
    assert!(api_env.contains("PUBLIC_URL='http://localhost:8080'"), "{}", api_env);
    assert!(api_env.contains("MACHINE_ID='dev-0'"), "{}", api_env);
}

#[test]
fn an_instance_shifts_every_named_port_and_renames_the_stack() {
    let dir = project(LOCALENV);
    let root = dir.path();
    assert_ok(&generate(root, &["--instance", "2"], &[]));

    assert!(!root.join("local_env").exists());
    let out = root.join("local_env_2");
    let parsed = compose(&out);
    assert_eq!(parsed["name"].as_str(), Some("shop_local_2"));
    // Container names are global to the daemon, so an instance leaves them to compose.
    assert!(parsed["services"]["api"]["container_name"].is_null());
    assert_eq!(parsed["services"]["api"]["ports"][0].as_str(), Some("14080:80"));
    // Ports in an extra spec file follow the instance too.
    assert_eq!(parsed["services"]["db"]["ports"][0].as_str(), Some("11432:5432"));

    let api_env = read(&out.join("api").join(".env"));
    assert!(api_env.contains("PUBLIC_URL='http://localhost:14080'"), "{}", api_env);
    // Only named ports move: the in-network address stays as written.
    assert!(api_env.contains("DB_URL='postgres://db:5432/shop'"), "{}", api_env);
    assert!(api_env.contains("MACHINE_ID='dev-2'"), "{}", api_env);

    let worker_env = read(&root.join("worker-src").join(".env"));
    assert!(
        worker_env.contains("DB_URL=postgres://localhost:11432/shop"),
        "{}",
        worker_env
    );
}

#[test]
fn the_instance_comes_from_the_environment_then_the_instance_file() {
    let dir = project(LOCALENV);
    let root = dir.path();
    fs::write(root.join(".simpled-instance"), "1\n").unwrap();

    assert_ok(&generate(root, &[], &[("SIMPLED_INSTANCE", "3")]));
    assert_eq!(
        compose(&root.join("local_env_3"))["name"].as_str(),
        Some("shop_local_3")
    );

    assert_ok(&generate(root, &[], &[]));
    assert_eq!(
        compose(&root.join("local_env_1"))["name"].as_str(),
        Some("shop_local_1")
    );

    // The flag wins over both.
    assert_ok(&generate(root, &["--instance", "0"], &[("SIMPLED_INSTANCE", "3")]));
    assert_eq!(compose(&root.join("local_env"))["name"].as_str(), Some("shop_local"));
}

#[test]
fn a_literal_host_port_is_rejected_above_instance_zero() {
    let dir = project(&LOCALENV.replace("\"$port(worker):80\"", "\"8081:80\""));
    let root = dir.path();
    assert_ok(&generate(root, &[], &[]));

    let output = generate(root, &["--instance", "1"], &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("host port 8081, which is not a named port"),
        "{}",
        stderr
    );
}

#[test]
fn a_literal_gateway_port_is_rejected_above_instance_zero() {
    let dir = project(&LOCALENV.replace("localhost:$port(web)\n", "localhost:8080\n"));
    let output = generate(dir.path(), &["--instance", "1"], &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Gateway host 'localhost:8080'"), "{}", stderr);
}

#[test]
fn an_unknown_port_name_is_an_error() {
    let dir = project(&LOCALENV.replace("$port(worker)", "$port(wrker)"));
    let output = generate(dir.path(), &[], &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("$port(wrker) names no port"), "{}", stderr);
}

#[test]
fn auto_is_refused_for_a_local_stack() {
    let dir = project(LOCALENV);
    let output = generate(dir.path(), &["--instance", "auto"], &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only for `simpled test`"));
}

const TESTSPEC: &str = r#"
suites:
  e2e:
    deployment:
      extends: dev
      environment:
        - PUBLIC_URL=http://localhost:$port(web)/from-suite
    working_dir: ./e2e
    environment:
      - E2E_URL=http://localhost:$port(web)/api
      - E2E_PUBLIC=${PUBLIC_URL}
      - E2E_ID=e2e-$instance()
    wait_for:
      - http://localhost:$port(web)/health
    run: RUN_COMMAND
"#;

#[test]
fn a_suite_sees_the_ports_of_the_instance_it_runs_as() {
    let dir = project(LOCALENV);
    let root = dir.path();
    let run = if cfg!(windows) {
        "set > vars.txt"
    } else {
        "env > vars.txt"
    };
    fs::write(root.join("testspec.yaml"), TESTSPEC.replace("RUN_COMMAND", run)).unwrap();
    fs::create_dir_all(root.join("e2e")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_simpled"))
        .current_dir(root)
        .env_remove("SIMPLED_INSTANCE")
        .args(["test", "e2e", "--no-up", "--instance", "1"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let vars = read(&root.join("e2e/vars.txt"));
    for expected in [
        "E2E_URL=http://localhost:11080/api",
        "E2E_PUBLIC=http://localhost:11080/from-suite",
        "E2E_ID=e2e-1",
    ] {
        assert!(vars.contains(expected), "{expected} missing:\n{vars}");
    }

    let output = Command::new(env!("CARGO_BIN_EXE_simpled"))
        .current_dir(root)
        .args(["test", "e2e", "--no-up", "--instance", "auto"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("with --no-up name the instance"));
}
