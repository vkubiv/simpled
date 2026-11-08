//! `simpled status`, `logs` and `wait` find the controller from the env spec. The
//! domains here do not resolve, so the errors show which URL was used.

mod common;

use common::{envspec, simpled, write_project};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";

fn project(controller: bool) -> tempfile::TempDir {
    let top = if controller {
        format!(
            "swarm_mode: true
controller:
  host: web
  read_token: \"{}\"",
            TOKEN
        )
    } else {
        "swarm_mode: true".to_string()
    };
    project_with(&top)
}

fn project_with(top: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), top);
    dir
}

fn write_env(root: &std::path::Path, top: &str) {
    let env = envspec("docker", top, "").replace("shop.example.com", "shop.simpled-test.invalid");
    std::fs::create_dir_all(root).unwrap();
    write_project(root, &env);
}

/// An env spec whose read token lives in Secrets Manager, as on a real cluster.
const AWS_TOKEN: &str = "swarm_mode: true
controller:
  host: web
  read_token:
    aws: dev/simpled-controller
    jq: .controller_read_token";

const REACHED: &str = "Cannot reach http://shop.simpled-test.invalid/simpled/api/v1/status";

fn run(dir: &tempfile::TempDir, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let output = simpled(dir.path(), args, env);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn status_needs_a_token() {
    let dir = project_with(
        "swarm_mode: true
controller:
  host: web",
    );
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    assert!(!ok);
    assert!(stderr.contains("Set SIMPLED_TOKEN"), "{}", stderr);
}

#[test]
fn status_reads_a_token_the_env_spec_holds() {
    let dir = project(true);
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn an_aws_token_without_a_secrets_file_says_what_to_write() {
    let dir = project_with(AWS_TOKEN);
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    assert!(!ok);
    assert!(stderr.contains("SIMPLED_TOKEN"), "{}", stderr);
    assert!(
        stderr.contains(
            r#"put it into a git-ignored secrets.json next to the env spec: {"dev/simpled-controller": {"controller_read_token": "..."}}"#
        ),
        "{}",
        stderr
    );
}

#[test]
fn an_aws_token_is_read_from_the_secrets_json_next_to_the_env_spec_only() {
    let top = tempfile::tempdir().unwrap();
    let env_dir = top.path().join("dev");
    write_env(&env_dir, AWS_TOKEN);
    let file = format!(
        r#"{{"dev/simpled-controller": {{"controller_read_token": "{}"}}}}"#,
        TOKEN
    );
    // One directory up is not looked at.
    std::fs::write(top.path().join("secrets.json"), &file).unwrap();
    let output = simpled(&env_dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("next to the env spec"), "{}", stderr);
    std::fs::write(env_dir.join("secrets.json"), &file).unwrap();
    let output = simpled(&env_dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn secrets_json_takes_the_secret_string_as_secrets_manager_returns_it() {
    let dir = project_with(AWS_TOKEN);
    let document = format!(r#"{{"controller_read_token": "{}"}}"#, TOKEN);
    let file = serde_json::json!({ "dev/simpled-controller": document });
    std::fs::write(dir.path().join("secrets.json"), file.to_string()).unwrap();
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn secrets_json_without_the_token_names_what_is_missing_and_never_the_content() {
    let dir = project_with(AWS_TOKEN);
    std::fs::write(
        dir.path().join("secrets.json"),
        r#"{"prod/simpled-controller": {"controller_read_token": "not-this-one-0123456789abcdef"}}"#,
    )
    .unwrap();
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", "")]);
    assert!(!ok);
    assert!(stderr.contains("has no value for controller.read_token"), "{}", stderr);
    assert!(!stderr.contains("not-this-one"), "{}", stderr);
}

#[test]
fn simpled_token_wins_over_secrets_json() {
    let dir = project_with(AWS_TOKEN);
    std::fs::write(dir.path().join("secrets.json"), "not json").unwrap();
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn status_finds_the_api_under_simpled_on_the_controller_host() {
    let dir = project(true);
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(
        stderr.contains("Cannot reach http://shop.simpled-test.invalid/simpled/api/v1/status"),
        "{}",
        stderr
    );
    let (_, stderr) = run(
        &dir,
        &[
            "logs",
            "prod",
            "api",
            "--url",
            "http://ops.simpled-test.invalid/simpled/api/v1/",
        ],
        &[("SIMPLED_TOKEN", TOKEN)],
    );
    assert!(
        stderr
            .contains("Cannot reach http://ops.simpled-test.invalid/simpled/api/v1/deployments/prod/services/api/logs"),
        "{}",
        stderr
    );
}

#[test]
fn wait_polls_the_deployments_health_on_the_controller_host_until_the_time_is_up() {
    let dir = project(false);
    let (ok, stderr) = run(
        &dir,
        &[
            "wait",
            "prod",
            "--version",
            "1.2.3",
            "--timeout",
            "1s",
            "--interval",
            "1s",
        ],
        &[],
    );
    assert!(!ok);
    assert!(
        stderr.contains("http://shop.simpled-test.invalid/simpled/health/prod did not report 1.2.3 as healthy within 1 seconds; last: cannot reach it"),
        "{}",
        stderr
    );
    let (ok, stderr) = run(&dir, &["wait", "staging", "--version", "1.2.3"], &[]);
    assert!(!ok);
    assert!(stderr.contains("staging"), "{}", stderr);
}
