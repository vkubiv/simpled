//! `aws-secrets.local.json`, the stand-in for AWS Secrets Manager on a machine
//! without AWS access: read by `simpled status` and `logs` for the controller's
//! token, never by `prepare-deployment` unless `--aws-secrets-file` asks for it.

mod common;

use common::{
    simpled, unreachable_project as project, write_unreachable_project as write_env, UNREACHABLE_STATUS as REACHED,
};
use std::path::{Path, PathBuf};

const TOKEN: &str = "0123456789abcdef0123456789abcdef-read";
const LOCAL_FILE: &str = "aws-secrets.local.json";

/// An env spec whose read token lives in Secrets Manager, as on a real cluster.
const AWS_TOKEN: &str = "swarm_mode: true
controller:
  host: web
  read_token:
    aws: dev/simpled-controller
    jq: .controller_read_token";

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
    assert!(
        stderr.contains(
            "the AWS secrets dev/simpled-controller were read from real.json and written into the prepared output"
        ),
        "{}",
        stderr
    );
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

#[test]
fn a_whitespace_only_simpled_token_counts_as_unset() {
    let dir = project(AWS_TOKEN);
    let empty = tempfile::tempdir().unwrap();
    let path = empty.path().to_str().unwrap().to_string();
    let output = simpled(dir.path(), &["status"], &[("SIMPLED_TOKEN", "   "), ("PATH", &path)]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("has no aws-secrets.local.json"), "{}", stderr);
}

#[test]
fn a_jq_dot_secret_cannot_be_stood_in_for() {
    let dir = project(&AWS_TOKEN.replace(".controller_read_token", "."));
    let (_, stderr) = status(dir.path());
    assert!(
        stderr.contains("it cannot hold this secret: its jq filter '.' is not a path to one field like .a.b"),
        "{}",
        stderr
    );
}

#[test]
fn a_jq_filter_beyond_a_path_gets_no_example() {
    let dir = project(&AWS_TOKEN.replace(".controller_read_token", "'.a | ascii_downcase'"));
    let (_, stderr) = status(dir.path());
    assert!(
        stderr.contains(
            "it cannot hold this secret: its jq filter '.a | ascii_downcase' is not a path to one field like .a.b"
        ),
        "{}",
        stderr
    );
    assert!(!stderr.contains("add it there"), "{}", stderr);
}

#[test]
fn a_broken_file_without_aws_names_both_problems() {
    let dir = project(AWS_TOKEN);
    std::fs::write(dir.path().join(LOCAL_FILE), r#"{"dev/simpled-controller": {"#).unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(
        stderr.contains("aws-secrets.local.json next to the env spec is unusable (")
            && stderr.contains("is not valid JSON")
            && stderr.contains("and the AWS CLI could not read it either. Fix the file, or remove it."),
        "{}",
        stderr
    );
    assert!(!stderr.contains("Secret 'dev/simpled-controller': "), "{}", stderr);
}

#[test]
fn aws_secrets_file_says_nothing_when_there_is_no_aws_secret() {
    let dir = project(&format!(
        "swarm_mode: true
controller:
  host: web
  read_token: \"{}\"",
        TOKEN
    ));
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
    assert!(ok, "{}", stderr);
    assert!(!stderr.contains("written into the prepared output"), "{}", stderr);
}

fn status_with_file(file: &str) -> String {
    let dir = project(AWS_TOKEN);
    std::fs::write(dir.path().join(LOCAL_FILE), file).unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    stderr
}

#[test]
fn a_false_or_null_field_fails_as_jq_e_would() {
    for value in ["false", "null"] {
        let stderr = status_with_file(&format!(
            r#"{{"dev/simpled-controller": {{"controller_read_token": {value}}}}}"#
        ));
        assert!(
            stderr.contains(&format!(
                ".controller_read_token is {value}, which jq -e treats as no value"
            )),
            "{}",
            stderr
        );
    }
}

#[test]
fn an_object_field_is_refused_rather_than_formatted_differently_from_jq() {
    let stderr = status_with_file(r#"{"dev/simpled-controller": {"controller_read_token": {"a": 1}}}"#);
    assert!(
        stderr.contains("is a JSON object, which jq -r prints indented"),
        "{}",
        stderr
    );
}

#[test]
fn a_number_field_is_printed_as_jq_prints_it() {
    // A number is no valid token, but it is read: the request is what fails.
    let stderr =
        status_with_file(r#"{"dev/simpled-controller": {"controller_read_token": 12345678901234567890123456789012}}"#);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn a_whole_secret_that_is_the_text_none_is_refused() {
    let dir = project(&AWS_TOKEN.replace("\n    jq: .controller_read_token", ""));
    std::fs::write(dir.path().join(LOCAL_FILE), r#"{"dev/simpled-controller": "None"}"#).unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(stderr.contains("is the text None"), "{}", stderr);
}

const READ_FROM_ENV_MANAGE_FROM_AWS: &str = "swarm_mode: true
controller:
  host: web
  read_token:
    env: SIMPLED_TEST_NO_SUCH_READ_TOKEN
  manage_token:
    aws: dev/simpled-controller
    jq: .controller_manage_token";

#[test]
fn the_manage_token_is_tried_when_the_read_token_cannot_be_read() {
    let dir = project(READ_FROM_ENV_MANAGE_FROM_AWS);
    std::fs::write(
        dir.path().join(LOCAL_FILE),
        format!(
            r#"{{"dev/simpled-controller": {{"controller_manage_token": "{}"}}}}"#,
            TOKEN
        ),
    )
    .unwrap();
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(stderr.contains(REACHED), "{}", stderr);
}

#[test]
fn when_neither_token_can_be_read_both_reasons_are_given() {
    let dir = project(READ_FROM_ENV_MANAGE_FROM_AWS);
    let (ok, stderr) = status(dir.path());
    assert!(!ok);
    assert!(
        stderr.contains("SIMPLED_TOKEN is not set, and neither token can be read here"),
        "{}",
        stderr
    );
    assert!(
        stderr.contains("controller.read_token cannot be read here"),
        "{}",
        stderr
    );
    assert!(
        stderr.contains("controller.manage_token cannot be read here"),
        "{}",
        stderr
    );
}
