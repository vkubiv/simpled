//! A broken spec must fail `prepare-deployment` with a non-zero exit and a
//! message that names the problem, before anything is generated.

mod common;

use common::{envspec, simpled, write_project};

fn stderr_of(args: &[&str], env: &[(&str, &str)], spec: &str) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), spec);
    let output = simpled(dir.path(), args, env);
    let generated = dir.path().join("manifests").exists() || dir.path().join("docker-deploy").exists();
    assert!(!generated, "nothing may be generated for a rejected spec");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_missing_secret_variable_fails_the_deploy_before_generation() {
    let (ok, stderr) = stderr_of(
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[],
        &envspec("k8s", "", ""),
    );
    assert!(!ok);
    assert!(
        stderr.contains("Secret environment variable SHOP_DB_PASSWORD not set"),
        "{stderr}"
    );
}

#[test]
fn an_unknown_deployment_lists_the_known_ones() {
    let (ok, stderr) = stderr_of(
        &["prepare-deployment", "staging", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "x")],
        &envspec("k8s", "", ""),
    );
    assert!(!ok);
    assert!(
        stderr.contains("'staging' not found") && stderr.contains("prod"),
        "{stderr}"
    );
}

#[test]
fn a_spec_that_fails_validation_names_the_rule() {
    // The env spec routes a service the application does not define.
    let env = envspec("k8s", "", "").replace(
        "    services:\n",
        "    services:\n      ghost:\n        host: web\n        prefix: /ghost\n",
    );
    let (ok, stderr) = stderr_of(
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "x")],
        &env,
    );
    assert!(!ok);
    assert!(stderr.contains("Validation failed"), "{stderr}");
    assert!(stderr.contains("configures service ghost"), "{stderr}");
}

#[test]
fn a_bundle_argument_is_required() {
    let (ok, stderr) = stderr_of(
        &["prepare-deployment", "prod"],
        &[("SHOP_DB_PASSWORD", "x")],
        &envspec("k8s", "", ""),
    );
    assert!(!ok);
    assert!(
        stderr.contains("--app-bundle") || stderr.contains("--download-bundle-from"),
        "{stderr}"
    );
}
