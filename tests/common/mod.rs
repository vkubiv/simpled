//! Fixture and runner shared by the end-to-end tests: a small application, an
//! environment for each target, and a way to run the real binary on them.

#![allow(dead_code)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

pub const APPSPEC: &str = r#"
name: shop
version: 1.2.3

environment:
  external:
    - LOG_LEVEL=info
    - PUBLIC_URL

secrets:
  - db_password

configs:
  data:
    - settings.json

app_services:
  api:
    type: public
    image: myorg/api
    environment:
      - $all
    secrets:
      - db_password:
          variable: DB_PASSWORD
    configs:
      - data: /data
    ports:
      - "80:8080"
    depends_on:
      - primary-db
  migrate:
    type: job
    image: myorg/migrate
    environment:
      - LOG_LEVEL
    secrets:
      - db_password:
          variable: DB_PASSWORD
    depends_on:
      - primary-db

extra_services:
  primary-db:
    type: internal
    image: postgres:16
    ports:
      - "5432:5432"
    healthcheck:
      test: ["CMD", "pg_isready"]
      interval: 5s
"#;

/// An env spec for `env_type` with one `prod` deployment of the fixture app.
pub fn envspec(env_type: &str, extra_top_level: &str, gateway_type: &str) -> String {
    format!(
        r#"
type: {env_type}
{extra_top_level}
gateway:
  hosts:
    web: shop.example.com
  tls:
    disable: true
  {gateway_type}

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
        env: SHOP_DB_PASSWORD
    configs:
      data: ./data
    services:
      api:
        host: web
        prefix: /
        replicas: 2
"#
    )
}

/// Writes the env spec, the app bundle directory and the config files into `root`.
pub fn write_project(root: &Path, env: &str) {
    fs::write(root.join("envspec.yaml"), env).unwrap();
    fs::create_dir_all(root.join("bundle")).unwrap();
    fs::write(root.join("bundle").join("appspec.yaml"), APPSPEC).unwrap();
    fs::create_dir_all(root.join("data")).unwrap();
    fs::write(root.join("data").join("settings.json"), "{}").unwrap();
}

/// Runs the built `simpled` binary in `root` with the given arguments and
/// environment variables.
pub fn simpled(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_simpled"));
    command.current_dir(root).args(args);
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("simpled runs")
}

/// Runs `prepare-deployment prod` on a freshly written project and returns the
/// directory, failing the test if the command did not succeed.
pub fn prepare(env: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write_project(dir.path(), env);
    let output = simpled(
        dir.path(),
        &["prepare-deployment", "prod", "--bundle", "bundle"],
        &[("SHOP_DB_PASSWORD", "s3cr3t")],
    );
    assert!(
        output.status.success(),
        "prepare-deployment failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}
