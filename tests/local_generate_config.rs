//! End-to-end check of `simpled local generate-config`, which needs no Docker:
//! it writes the local compose file, the per-service env files and the
//! working directory of a host-run service, and those are inspected here.

use std::fs;
use std::path::Path;
use std::process::Command;

const APPSPEC: &str = r#"
name: shop
version: 1.2.3

environment:
  external:
    - PUBLIC_URL
    - DB_HOST=primary-db

secrets:
  - db_password

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
    depends_on:
      - primary-db
  worker:
    type: internal
    image: myorg/worker
    environment:
      - $all
    secrets:
      - db_password:
          variable: DB_PASSWORD
    ports:
      - "80:8080"
  migrate:
    type: job
    image: myorg/migrate
    environment:
      - DB_HOST
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
"#;

const LOCALENV: &str = r#"
gateway:
  hosts:
    web: localhost:8080

deployments:
  dev:
    primary_host: web
    application:
      name: shop
    environment:
      - PUBLIC_URL=http://localhost:8080
    undockerized_environment:
      - DB_HOST=localhost
    secrets:
      db_password: local pa$$ 'word'
    services:
      api:
        host: web
        prefix: /
        ports:
          - "8080:80"
      worker:
        ports:
          - "8081:80"
        working_dir: ./worker-src
"#;

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

#[test]
fn local_config_covers_compose_env_files_and_working_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("localenv.yaml"), LOCALENV).unwrap();
    fs::write(root.join("appspec.yaml"), APPSPEC).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_simpled"))
        .current_dir(root)
        .args(["local", "generate-config"])
        .output()
        .expect("simpled runs");
    assert!(
        output.status.success(),
        "generate-config failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let local_env = root.join("local_env");
    let compose = read(&local_env.join("docker-compose.yaml"));
    let parsed: serde_yaml::Value = serde_yaml::from_str(&compose).unwrap();

    // One compose project per application, so two apps on one machine do not
    // remove each other's containers as orphans.
    assert_eq!(parsed["name"].as_str(), Some("shop_local"), "{}", compose);
    // Locally every container is named, and reads its env file from the tree.
    assert_eq!(
        parsed["services"]["api"]["container_name"].as_str(),
        Some("api"),
        "{}",
        compose
    );
    assert_eq!(
        parsed["services"]["api"]["env_file"][0].as_str(),
        Some("./api/.env"),
        "{}",
        compose
    );
    assert_eq!(
        parsed["services"]["api"]["ports"][0].as_str(),
        Some("8080:80"),
        "{}",
        compose
    );
    // A local app image is the untagged build on this machine.
    assert_eq!(
        parsed["services"]["api"]["image"].as_str(),
        Some("myorg/api:latest"),
        "{}",
        compose
    );
    // compose honours depends_on, and waits for health where a check exists.
    assert_eq!(
        parsed["services"]["migrate"]["depends_on"]["primary-db"]["condition"].as_str(),
        Some("service_healthy"),
        "{}",
        compose
    );

    // The compose env file is single-quoted, since compose would otherwise
    // expand `$` and treat ` #` as a comment.
    let api_env = read(&local_env.join("api").join(".env"));
    assert!(api_env.contains("PUBLIC_URL='http://localhost:8080'"), "{}", api_env);
    assert!(api_env.contains("DB_HOST='primary-db'"), "{}", api_env);
    // The secret is inlined by compose too, so it goes through `environment:`.
    assert_eq!(
        parsed["services"]["api"]["environment"]["DB_PASSWORD"].as_str(),
        Some("local pa$$ 'word'"),
        "{}",
        compose
    );

    // The undockerized file for a dockerized service is plain dotenv.
    let api_undockerized = read(&local_env.join("api").join("undockerized.env"));
    assert!(api_undockerized.contains("DB_HOST=localhost"), "{}", api_undockerized);

    // A host-run service gets its `.env` and secrets in working_dir instead.
    let worker_env = read(&root.join("worker-src").join(".env"));
    assert!(worker_env.contains("DB_HOST=localhost"), "{}", worker_env);
    assert!(worker_env.contains("DB_PASSWORD=local pa$$ 'word'"), "{}", worker_env);
    assert!(!local_env.join("worker").join("undockerized.env").exists());
}
