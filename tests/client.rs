//! `simpled status`, `logs` and `wait` find the controller from the env spec. The
//! domains here do not resolve, so the errors show which URL was used.

mod common;

use common::{simpled, unreachable_project, UNREACHABLE_STATUS as REACHED};

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
    unreachable_project(top)
}

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
fn logs_of_the_gateway_take_no_service() {
    let dir = project(true);
    let (ok, stderr) = run(&dir, &["logs", "gateway", "--tail", "50"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(
        stderr.contains("Cannot reach http://shop.simpled-test.invalid/simpled/api/v1/gateway/logs"),
        "{}",
        stderr
    );

    let (ok, stderr) = run(&dir, &["logs", "prod"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(
        stderr.contains("Name the service too: simpled logs prod <service>"),
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
