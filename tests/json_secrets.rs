//! `json_file` and `json_env`: secrets that are fields of one JSON document,
//! read when the deployment is prepared.

mod common;

use common::*;
use std::fs;
use std::path::Path;

/// The fixture app twice, side by side: `prod` reads its secret from a JSON
/// document, `other` from a document of its own that is not there.
fn two_deployments(prod_secret: &str) -> String {
    format!(
        r#"
type: docker
gateway:
  hosts:
    web: shop.example.com
    other: other.example.com
  tls:
    disable: true
  type: traefik

registry:
  myorg: registry.example.com

deployments:
  prod:
    primary_host: web
    application:
      name: shop
    environment:
      - PUBLIC_URL=https://shop.example.com
    secrets:
      db_password:
{prod_secret}
    configs:
      data: ./data
    services:
      api:
        host: web
        prefix: /
  other:
    primary_host: other
    application:
      name: shop
    environment:
      - PUBLIC_URL=https://other.example.com
    secrets:
      db_password:
        json_file: ./not-there.json
    configs:
      data: ./data
    services:
      api:
        host: other
        prefix: /
"#
    )
}

fn run(prod_secret: &str, files: &[(&str, &str)], env: &[(&str, &str)]) -> (tempfile::TempDir, std::process::Output) {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), &two_deployments(prod_secret));
    for (name, content) in files {
        fs::write(dir.path().join(name), content).unwrap();
    }
    let output = simpled(dir.path(), &["prepare-deployment", "prod", "--bundle", "bundle"], env);
    (dir, output)
}

fn secret(root: &Path) -> String {
    read(&root.join("docker-deploy").join("secrets").join("shop-db_password"))
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace('\\', "/")
}

#[test]
fn json_env_reads_the_field_named_after_the_secret() {
    let (dir, output) = run(
        "        json_env: SHOP_SECRETS",
        &[],
        &[("SHOP_SECRETS", r#"{"db_password": "from-env\n", "unused": "x"}"#)],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(secret(dir.path()), "from-env");
}

/// `other` names a file that does not exist; preparing `prod` never reads it.
#[test]
fn json_file_with_jq_is_read_only_for_the_deployment_being_prepared() {
    let (dir, output) = run(
        "        json_file: ./shop.json\n        jq: .db.password",
        &[("shop.json", r#"{"db": {"password": "from-file"}}"#)],
        &[],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(secret(dir.path()), "from-file");
}

#[test]
fn a_missing_document_or_field_stops_the_deployment_by_name() {
    let (_, unset) = run("        json_env: SHOP_SECRETS", &[], &[]);
    assert!(!unset.status.success());
    assert!(
        stderr(&unset).contains("Secret 'db_password' reads json_env SHOP_SECRETS, which is not set"),
        "{}",
        stderr(&unset)
    );

    let (_, no_field) = run(
        "        json_env: SHOP_SECRETS",
        &[],
        &[("SHOP_SECRETS", r#"{"other": "1"}"#)],
    );
    assert!(
        stderr(&no_field).contains("Secret 'db_password' has no value: $SHOP_SECRETS has no .db_password"),
        "{}",
        stderr(&no_field)
    );

    let (_, empty) = run(
        "        json_env: SHOP_SECRETS",
        &[],
        &[("SHOP_SECRETS", r#"{"db_password": ""}"#)],
    );
    assert!(
        stderr(&empty).contains("Secret 'db_password' is empty in $SHOP_SECRETS"),
        "{}",
        stderr(&empty)
    );

    let (_, no_file) = run("        json_file: ./shop.json", &[], &[]);
    assert!(
        stderr(&no_file).contains("Secret 'db_password' reads json_file shop.json, which does not exist"),
        "{}",
        stderr(&no_file)
    );
}

/// A parse error points at the position, never at the content: it is the secret.
#[test]
fn invalid_json_is_reported_without_its_content() {
    let (_, output) = run(
        "        json_env: SHOP_SECRETS",
        &[],
        &[("SHOP_SECRETS", r#"{"db_password": "hunter2""#)],
    );
    let message = stderr(&output);
    assert!(message.contains("$SHOP_SECRETS is not valid JSON (line 1"), "{message}");
    assert!(!message.contains("hunter2"), "{message}");
}
