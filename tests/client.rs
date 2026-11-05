//! `simpled status`, `logs` and `wait` find the controller from the env spec. The
//! domains here do not resolve, so the errors show which URL was used.

mod common;

use common::{envspec, simpled, write_project};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";

fn project(controller: bool) -> tempfile::TempDir {
    let top = if controller {
        format!(
            "swarm_mode: true\ncontroller:\n  hosts:\n    web:\n      prefix: /_simpled\n  read_token: \"{}\"",
            TOKEN
        )
    } else {
        "swarm_mode: true".to_string()
    };
    let env = envspec("docker", &top, "").replace("shop.example.com", "shop.simpled-test.invalid");
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), &env);
    dir
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
    let dir = project(true);
    let (ok, stderr) = run(&dir, &["status"], &[]);
    assert!(!ok);
    assert!(stderr.contains("Set SIMPLED_TOKEN"), "{}", stderr);
}

#[test]
fn status_finds_the_api_under_its_prefix() {
    let dir = project(true);
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(
        stderr.contains("Cannot reach http://shop.simpled-test.invalid/_simpled/api/v1/status"),
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
            "http://ops.simpled-test.invalid/api/v1/",
        ],
        &[("SIMPLED_TOKEN", TOKEN)],
    );
    assert!(
        stderr.contains("Cannot reach http://ops.simpled-test.invalid/api/v1/deployments/prod/services/api/logs"),
        "{}",
        stderr
    );
}

#[test]
fn without_controller_hosts_status_says_how_to_route_the_api() {
    let dir = project(false);
    let (ok, stderr) = run(&dir, &["status"], &[("SIMPLED_TOKEN", TOKEN)]);
    assert!(!ok);
    assert!(stderr.contains("set controller.hosts"), "{}", stderr);
}

#[test]
fn wait_polls_the_primary_hosts_health_until_the_time_is_up() {
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
        stderr.contains("http://shop.simpled-test.invalid/.well-known/simpled/health did not report 1.2.3 as healthy within 1 seconds; last: cannot reach it"),
        "{}",
        stderr
    );
    let (ok, stderr) = run(&dir, &["wait", "staging", "--version", "1.2.3"], &[]);
    assert!(!ok);
    assert!(stderr.contains("staging"), "{}", stderr);
}
