//! `exclude_services` and `simpled test` on a fixture that needs no Docker:
//! `local generate-config` shows what an exclusion keeps and drops, and
//! `simpled test --no-up` runs a suite's command with the environment and
//! secrets it was promised, and exits with the command's own status.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const APPSPEC: &str = r#"
name: shop
version: 1.2.3

environment:
  external:
    - PUBLIC_URL
    - DB_HOST=primary-db

secrets:
  - db_password
  - api_key

app_services:
  api:
    type: public
    image: myorg/api
    environment:
      - $all
    secrets:
      - db_password:
          variable: DB_PASSWORD
    ports:
      - "80:8080"
  web:
    type: public
    image: myorg/web
    ports:
      - "80:8080"

extra_services:
  primary-db:
    type: internal
    image: postgres:16
    ports:
      - "5432:5432"
"#;

const LOCALENV: &str = r#"
gateway:
  hosts:
    web: localhost:8080

deployments:
  local:
    primary_host: web
    application:
      name: shop
    environment:
      - PUBLIC_URL=http://localhost:8080
    undockerized_environment:
      - DB_HOST=localhost
    secrets:
      db_password: pw
      api_key: k3y
    services:
      api:
        host: web
        prefix: /api
        ports:
          - "8081:80"
        working_dir: ./api-src
      web:
        host: web
        prefix: /
        ports:
          - "8082:80"
  infra:
    extends: local
    exclude_services: [api, web]
"#;

const TESTSPEC: &str = r#"
suites:
  e2e:
    deployment:
      extends: local
      exclude_services: [web]
      environment:
        - PUBLIC_URL=http://localhost:9999
    working_dir: ./e2e
    environment:
      - DB_HOST
      - E2E_URL=${PUBLIC_URL}/api
    secrets:
      - db_password:
          variable: PGPASSWORD
      - api_key
    run: RUN_COMMAND
  failing:
    deployment: local
    run: exit 3
"#;

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

fn simpled(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simpled"))
        .current_dir(root)
        .args(args)
        .output()
        .expect("simpled runs")
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success(),
        "simpled failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The shell `simpled test` uses, dumping the process environment to a file.
fn dump_env_command() -> &'static str {
    if cfg!(windows) {
        "set > vars.txt"
    } else {
        "env > vars.txt"
    }
}

fn write_project(root: &Path) {
    fs::write(root.join("localenv.yaml"), LOCALENV).unwrap();
    fs::write(root.join("appspec.yaml"), APPSPEC).unwrap();
    fs::write(
        root.join("testspec.yaml"),
        TESTSPEC.replace("RUN_COMMAND", dump_env_command()),
    )
    .unwrap();
    fs::create_dir_all(root.join("e2e")).unwrap();
}

#[test]
fn an_excluded_service_keeps_its_working_dir_but_leaves_the_compose_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root);

    assert_ok(&simpled(root, &["local", "generate-config", "--deployment", "infra"]));

    let compose: serde_yaml::Value = serde_yaml::from_str(&read(&root.join("local_env/docker-compose.yaml"))).unwrap();
    let services = compose["services"].as_mapping().unwrap();
    assert!(services.contains_key("primary-db"), "{:?}", services);
    assert!(!services.contains_key("api"), "{:?}", services);
    assert!(!services.contains_key("web"), "{:?}", services);

    // The developer runs `api` by hand, so its env and secrets are still written.
    let api_env = read(&root.join("api-src/.env"));
    assert!(api_env.contains("DB_HOST=localhost"), "{api_env}");
    assert!(api_env.contains("DB_PASSWORD=pw"), "{api_env}");
}

#[test]
fn an_unknown_excluded_service_is_rejected_on_the_command_line_too() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root);

    let output = simpled(root, &["local", "run", "--deployment", "local", "--exclude", "nope"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--exclude names service 'nope'"), "{stderr}");
    assert!(stderr.contains("api, primary-db, web"), "{stderr}");
    assert!(
        !root.join("local_env").exists(),
        "nothing is generated for a bad exclusion"
    );
}

#[test]
fn a_suite_runs_with_its_environment_and_secrets_against_its_own_deployment() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root);

    assert_ok(&simpled(root, &["test", "e2e", "--no-up"]));

    // `.env` for the suite's own tooling, and the same variables in the process.
    let env_file = read(&root.join("e2e/.env"));
    let process_env = read(&root.join("e2e/vars.txt"));
    for expected in [
        // Forwarded from the deployment's host view: the undockerized override.
        "DB_HOST=localhost",
        // Substituted from the suite's inline deployment, which overrides `local`.
        "E2E_URL=http://localhost:9999/api",
        "PGPASSWORD=pw",
    ] {
        assert!(env_file.contains(expected), "{expected} missing from .env:\n{env_file}");
        assert!(
            process_env.contains(expected),
            "{expected} missing from process env:\n{process_env}"
        );
    }
    // A bare secret is a file under the working directory.
    assert_eq!(read(&root.join("e2e/secrets/api_key")), "k3y");
    // Only what the suite asked for is forwarded.
    assert!(!env_file.contains("PUBLIC_URL="), "{env_file}");
    // `--no-up` touches no compose project.
    assert!(!root.join("test_env").exists());
}

#[test]
fn a_failing_suite_sets_the_exit_code_and_a_run_of_every_suite_reports_the_first_failure() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root);

    let output = simpled(root, &["test", "failing", "--no-up"]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Suite 'failing' failed with exit code 3"), "{stdout}");

    // Every suite in name order: e2e passes, failing fails, and the run says so.
    let output = simpled(root, &["test", "--no-up"]);
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Suite 'e2e' passed"), "{stdout}");
    assert!(stdout.contains("Suite 'failing' failed with exit code 3"), "{stdout}");
    assert!(root.join("e2e/vars.txt").exists());
}

#[test]
fn a_suite_is_checked_against_its_deployment_before_anything_runs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_project(root);

    let stderr_of = |testspec: &str| {
        fs::write(root.join("testspec.yaml"), testspec).unwrap();
        let output = simpled(root, &["test", "--no-up"]);
        assert!(!output.status.success());
        String::from_utf8_lossy(&output.stderr).into_owned()
    };

    let err = stderr_of("suites:\n  e2e:\n    deployment: local\n    environment: [NOPE]\n    run: x\n");
    assert!(
        err.contains("Suite 'e2e' forwards NOPE, which deployment 'local' does not define"),
        "{err}"
    );

    let err = stderr_of("suites:\n  e2e:\n    deployment: local\n    secrets: [nope]\n    run: x\n");
    assert!(
        err.contains("Suite 'e2e' uses secret 'nope', which deployment 'local' does not provide"),
        "{err}"
    );

    let err = stderr_of("suites:\n  local:\n    deployment:\n      extends: local\n    run: x\n");
    assert!(err.contains("already has a deployment named 'local'"), "{err}");

    let err = stderr_of("suites:\n  e2e:\n    run: x\n");
    assert!(
        err.contains("does not name a deployment and the env spec defines several"),
        "{err}"
    );

    let err = stderr_of("suites:\n  e2e:\n    deployment: local\n    working_dir: ./missing\n    run: x\n");
    assert!(err.contains("working_dir"), "{err}");
    assert!(err.contains("not a directory"), "{err}");

    let err = stderr_of("suites:\n  e2e:\n    deployment: staging\n    run: x\n");
    assert!(err.contains("Deployment 'staging' not found"), "{err}");

    let output = simpled(root, &["test", "nope", "--no-up"]);
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("Suite 'nope' not found in testspec.yaml"), "{err}");
}
