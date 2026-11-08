//! `aws-secrets.local.json`, the stand-in for AWS Secrets Manager on a machine
//! without AWS access: read by `simpled status` and `logs` for the controller's
//! token, never by `prepare-deployment` unless `--aws-secrets-file` asks for it.

mod common;

use common::{envspec, simpled, write_project};
use std::path::{Path, PathBuf};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";
const LOCAL_FILE: &str = "aws-secrets.local.json";
const REACHED: &str = "Cannot reach http://shop.simpled-test.invalid/simpled/api/v1/status";

/// An env spec whose read token lives in Secrets Manager, as on a real cluster.
const AWS_TOKEN: &str = "swarm_mode: true
controller:
  host: web
  read_token:
    aws: dev/simpled-controller
    jq: .controller_read_token";

fn write_env(root: &Path, top: &str) {
    let env = envspec("docker", top, "").replace("shop.example.com", "shop.simpled-test.invalid");
    std::fs::create_dir_all(root).unwrap();
    write_project(root, &env);
}

fn project(top: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_env(dir.path(), top);
    dir
}

fn local_file(token: &str) -> String {
    format!(
        r#"{{"dev/simpled-controller": {{"controller_read_token": "{}"}}}}"#,
        token
    )
}

/// Runs simpled with no SIMPLED_TOKEN and no AWS CLI to fall back on: PATH is an
/// empty directory, so a test never reaches the developer's own AWS account.
fn run_without_aws(dir: &Path, args: &[&str]) -> (bool, String) {
    let empty = tempfile::tempdir().unwrap();
    let path = empty.path().to_str().unwrap().to_string();
    let output = simpled(
        dir,
        args,
        &[("SIMPLED_TOKEN", ""), ("PATH", &path), ("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn status(dir: &Path) -> (bool, String) {
    run_without_aws(dir, &["status"])
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(files_under(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn output_holds(dir: &Path, needle: &str) -> bool {
    files_under(&dir.join("docker-deploy"))
        .iter()
        .any(|f| String::from_utf8_lossy(&std::fs::read(f).unwrap()).contains(needle))
}

#[test]
fn without_the_file_or_aws_the_error_says_what_to_write() {
    let dir = project(AWS_TOKEN);
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(stderr.contains("SIMPLED_TOKEN is not set"), "{}", stderr);
    assert!(
        stderr.contains(
            r#"has no aws-secrets.local.json next to the env spec, and the AWS CLI could not read it. Without AWS access, add it there: {"dev/simpled-controller":{"controller_read_token":"..."}}"#
        ),
        "{}",
        stderr
    );
}

#[test]
fn the_example_nests_a_nested_jq_path() {
    let dir = project(&AWS_TOKEN.replace(".controller_read_token", ".tokens.read"));
    let (_, stderr) = status(dir.path());
    assert!(
        stderr.contains(r#"{"dev/simpled-controller":{"tokens":{"read":"..."}}}"#),
        "{}",
        stderr
    );
}

#[test]
fn the_token_is_read_from_the_file_next_to_the_env_spec_only() {
    let top = tempfile::tempdir().unwrap();
    let env_dir = top.path().join("dev");
    write_env(&env_dir, AWS_TOKEN);
    // One directory up is not looked at.
    std::fs::write(top.path().join(LOCAL_FILE), local_file(TOKEN)).unwrap();
    let (_, stderr) = status(&env_dir);
    assert!(stderr.contains("has no aws-secrets.local.json"), "{}", stderr);

    std::fs::write(env_dir.join(LOCAL_FILE), local_file(TOKEN)).unwrap();
    let (ok, stderr) = status(&env_dir);
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn the_file_takes_the_secret_string_as_secrets_manager_returns_it() {
    let dir = project(AWS_TOKEN);
    let document = format!(r#"{{"controller_read_token": "{}"}}"#, TOKEN);
    let file = serde_json::json!({ "dev/simpled-controller": document });
    std::fs::write(dir.path().join(LOCAL_FILE), file.to_string()).unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn a_secret_the_file_does_not_list_falls_back_to_aws() {
    let dir = project(AWS_TOKEN);
    std::fs::write(
        dir.path().join(LOCAL_FILE),
        r#"{"prod/simpled-controller": {"controller_read_token": "not-this-one-0123456789abcdef"}}"#,
    )
    .unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(
        stderr.contains(
            "is not listed in aws-secrets.local.json next to the env spec, and the AWS CLI could not read it"
        ),
        "{}",
        stderr
    );
    assert!(!stderr.contains("not-this-one"), "{}", stderr);
}

#[test]
fn a_listed_secret_without_the_field_is_an_error_not_a_fallback() {
    let dir = project(AWS_TOKEN);
    std::fs::write(
        dir.path().join(LOCAL_FILE),
        r#"{"dev/simpled-controller": {"controller_manage_token": "not-this-one-0123456789abcdef"}}"#,
    )
    .unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(
        stderr.contains("dev/simpled-controller has no .controller_read_token"),
        "{}",
        stderr
    );
    assert!(!stderr.contains("AWS CLI"), "{}", stderr);
    assert!(!stderr.contains("not-this-one"), "{}", stderr);
}

#[test]
fn a_whole_secret_given_as_a_document_says_so() {
    let dir = project(&AWS_TOKEN.replace("\n    jq: .controller_read_token", ""));
    std::fs::write(dir.path().join(LOCAL_FILE), local_file(TOKEN)).unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(
        stderr.contains("is a JSON document, but the spec reads the whole secret"),
        "{}",
        stderr
    );
}

#[test]
fn simpled_token_wins_over_the_file() {
    let dir = project(AWS_TOKEN);
    std::fs::write(dir.path().join(LOCAL_FILE), "not json").unwrap();
    let output = simpled(dir.path(), &["status"], &[("SIMPLED_TOKEN", TOKEN)]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn with_url_a_broken_env_spec_is_reported_as_such() {
    let dir = project(AWS_TOKEN);
    std::fs::write(dir.path().join("envspec.yaml"), "type: docker\ngateway: [").unwrap();
    let (ok, stderr) = run_without_aws(
        dir.path(),
        &["status", "--url", "http://ops.simpled-test.invalid/simpled/api/v1"],
    );
    assert!(!ok);
    assert!(
        stderr.contains("the env spec here, which would name the token, cannot be read"),
        "{}",
        stderr
    );
    assert!(stderr.contains("Failed to parse"), "{}", stderr);
}

#[test]
fn with_url_and_simpled_token_no_env_spec_is_needed() {
    let dir = tempfile::tempdir().unwrap();
    let output = simpled(
        dir.path(),
        &["status", "--url", "http://ops.simpled-test.invalid/simpled/api/v1"],
        &[("SIMPLED_TOKEN", TOKEN)],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Cannot reach http://ops.simpled-test.invalid/simpled/api/v1/status"),
        "{}",
        stderr
    );
}

#[test]
fn prepare_deployment_warns_that_the_file_is_not_deployed() {
    let dir = project(AWS_TOKEN);
    std::fs::write(
        dir.path().join(LOCAL_FILE),
        local_file("local-only-0123456789abcdef0123456789"),
    )
    .unwrap();
    let (ok, stderr) = run_without_aws(dir.path(), &["prepare-deployment", "prod", "--bundle", "bundle"]);
    assert!(ok, "{}", stderr);
    assert!(
        stderr.contains(
            "Warning: aws-secrets.local.json is not used by prepare-deployment: the AWS secrets dev/simpled-controller are fetched on the deploy target by fetch-secrets.sh. To write its values into the output instead, pass --aws-secrets-file aws-secrets.local.json."
        ),
        "{}",
        stderr
    );
    assert!(!output_holds(dir.path(), "local-only-"));
}

#[test]
fn prepare_deployment_says_nothing_without_the_file() {
    let dir = project(AWS_TOKEN);
    let (ok, stderr) = run_without_aws(dir.path(), &["prepare-deployment", "prod", "--bundle", "bundle"]);
    assert!(ok, "{}", stderr);
    assert!(!stderr.contains(LOCAL_FILE), "{}", stderr);
}

#[test]
fn prepare_deployment_with_aws_secrets_file_writes_its_values_into_the_output() {
    let dir = project(AWS_TOKEN);
    std::fs::write(
        dir.path().join("real.json"),
        local_file("from-the-file-0123456789abcdef0123456789"),
    )
    .unwrap();
    let (ok, stderr) = run_without_aws(
        dir.path(),
        &[
            "prepare-deployment",
            "prod",
            "--bundle",
            "bundle",
            "--aws-secrets-file",
            "real.json",
        ],
    );
    assert!(ok, "{}", stderr);
    assert!(stderr.contains("are written into the prepared output"), "{}", stderr);
    assert!(output_holds(dir.path(), "from-the-file-"));
    assert!(!dir.path().join("docker-deploy").join("fetch-secrets.sh").exists());
}

#[test]
fn prepare_deployment_with_aws_secrets_file_needs_every_secret_in_it() {
    let dir = project(AWS_TOKEN);
    std::fs::write(dir.path().join("real.json"), "{}").unwrap();
    let (ok, stderr) = run_without_aws(
        dir.path(),
        &[
            "prepare-deployment",
            "prod",
            "--bundle",
            "bundle",
            "--aws-secrets-file",
            "real.json",
        ],
    );
    assert!(!ok);
    assert!(
        stderr.contains("Secret 'dev/simpled-controller' is not listed in real.json (--aws-secrets-file)"),
        "{}",
        stderr
    );
}

#[test]
fn prepare_deployment_with_a_missing_aws_secrets_file_fails() {
    let dir = project(AWS_TOKEN);
    let (ok, stderr) = run_without_aws(
        dir.path(),
        &[
            "prepare-deployment",
            "prod",
            "--bundle",
            "bundle",
            "--aws-secrets-file",
            "nope.json",
        ],
    );
    assert!(!ok);
    assert!(
        stderr.contains("--aws-secrets-file nope.json: no such file"),
        "{}",
        stderr
    );
}
