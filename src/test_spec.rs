//! `testspec.yaml`: the suites `simpled test` runs against a local deployment.
//!
//! A suite is a host-run process in all but name, so its `environment:` and
//! `secrets:` use the grammar a service uses in `appspec.yaml`, and its
//! `deployment:` is either the name of a deployment in `localenv.yaml` or a
//! deployment block of its own, usually one that `extends` a named one.

use crate::spec::{parse_duration_secs, ServiceEnvOption, ServiceSecret};
use crate::spec_yaml::{DeploymentSpecYaml, ServiceSecretYaml};
use crate::transform;
use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize)]
pub struct TestSpecYaml {
    pub suites: HashMap<String, TestSuiteYaml>,
}

#[derive(Debug, Deserialize)]
pub struct TestSuiteYaml {
    pub deployment: Option<SuiteDeploymentYaml>,
    pub working_dir: Option<String>,
    pub environment: Option<Vec<String>>,
    pub secrets: Option<Vec<ServiceSecretYaml>>,
    pub wait_for: Option<Vec<String>>,
    pub timeout: Option<String>,
    pub run: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SuiteDeploymentYaml {
    Named(String),
    Inline(Box<DeploymentSpecYaml>),
}

#[derive(Debug)]
pub enum SuiteDeployment {
    /// The env spec's only deployment.
    Default,
    Named(String),
    /// A deployment declared by the suite itself, registered under the suite's name.
    Inline(Box<DeploymentSpecYaml>),
}

#[derive(Debug)]
pub struct TestSuite {
    pub name: String,
    pub deployment: SuiteDeployment,
    pub working_dir: PathBuf,
    pub environment: Vec<ServiceEnvOption>,
    pub secrets: Vec<ServiceSecret>,
    pub wait_for: Vec<String>,
    pub timeout: Duration,
    pub run: String,
}

impl TestSuite {
    /// The deployment this suite runs, as selected in the env spec.
    pub fn deployment_name(&self) -> Option<&str> {
        match &self.deployment {
            SuiteDeployment::Default => None,
            SuiteDeployment::Named(name) => Some(name),
            SuiteDeployment::Inline(_) => Some(&self.name),
        }
    }
}

pub fn load_test_spec(root: &Path) -> Result<Vec<TestSuite>> {
    let path = ["testspec.yaml", "testspec.yml"]
        .iter()
        .map(|name| root.join(name))
        .find(|p| p.exists())
        .ok_or_else(|| anyhow!("Could not find testspec.yaml or testspec.yml in {:?}", root))?;
    let file = File::open(&path).with_context(|| format!("Failed to open {:?}", path))?;
    let yaml: TestSpecYaml = serde_yaml::from_reader(file).with_context(|| format!("Failed to parse {:?}", path))?;
    convert_test_spec(yaml, root).context("Failed to process test spec")
}

/// Suites in name order, so a run without a suite name is stable.
pub fn convert_test_spec(yaml: TestSpecYaml, root: &Path) -> Result<Vec<TestSuite>> {
    if yaml.suites.is_empty() {
        bail!("testspec.yaml defines no suites");
    }
    let mut names: Vec<&String> = yaml.suites.keys().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| convert_suite(name, &yaml.suites[name], root))
        .collect()
}

fn convert_suite(name: &str, yaml: &TestSuiteYaml, root: &Path) -> Result<TestSuite> {
    let run = yaml
        .run
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| anyhow!("Suite '{}' has no 'run' command", name))?
        .to_string();

    let deployment = match &yaml.deployment {
        None => SuiteDeployment::Default,
        Some(SuiteDeploymentYaml::Named(dep)) => SuiteDeployment::Named(dep.clone()),
        Some(SuiteDeploymentYaml::Inline(dep)) => {
            if dep.is_abstract == Some(true) {
                bail!(
                    "Suite '{}' declares an abstract deployment; a suite's deployment is what runs",
                    name
                );
            }
            SuiteDeployment::Inline(dep.clone())
        }
    };

    let timeout = match yaml.timeout.as_deref() {
        None => DEFAULT_TIMEOUT,
        Some(raw) => Duration::from_secs(parse_duration_secs(raw).ok_or_else(|| {
            anyhow!(
                "Suite '{}' has an invalid timeout '{}': expected a duration such as \"90s\" or \"2m\"",
                name,
                raw
            )
        })?),
    };

    let environment = yaml
        .environment
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(transform::parse_service_env_option)
        .collect();

    let secrets = transform::convert_service_secrets(yaml.secrets.clone().unwrap_or_default())
        .with_context(|| format!("Suite '{}' has an invalid secrets list", name))?;

    Ok(TestSuite {
        name: name.to_string(),
        deployment,
        working_dir: root.join(yaml.working_dir.as_deref().unwrap_or(".")),
        environment,
        secrets,
        wait_for: yaml.wait_for.clone().unwrap_or_default(),
        timeout,
        run,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::SecretMount;

    fn convert(raw: &str) -> Result<Vec<TestSuite>> {
        let yaml: TestSpecYaml = serde_yaml::from_str(raw).unwrap();
        convert_test_spec(yaml, Path::new("/project"))
    }

    #[test]
    fn a_suite_names_a_deployment_or_declares_one() {
        let suites = convert(
            r#"
suites:
  smoke:
    deployment: local
    run: npm run smoke
  e2e:
    deployment:
      extends: local
      environment:
        - NODE_ENV=test
    working_dir: ./e2e
    environment:
      - $all
      - API_KEY
      - E2E_URL=${PUBLIC_URL}/api
    secrets:
      - db_password:
          variable: PGPASSWORD
    wait_for:
      - http://localhost:8080/health
    timeout: 1m30s
    run: npm test
"#,
        )
        .unwrap();

        // Name order, whatever the file order was.
        assert_eq!(suites[0].name, "e2e");
        assert_eq!(suites[1].name, "smoke");

        let e2e = &suites[0];
        assert!(matches!(&e2e.deployment, SuiteDeployment::Inline(d) if d.extends.as_deref() == Some("local")));
        assert_eq!(e2e.deployment_name(), Some("e2e"));
        assert_eq!(e2e.working_dir, Path::new("/project").join("./e2e"));
        assert_eq!(e2e.timeout, Duration::from_secs(90));
        assert_eq!(e2e.wait_for, vec!["http://localhost:8080/health"]);
        assert_eq!(e2e.run, "npm test");
        assert!(matches!(e2e.environment[0], ServiceEnvOption::All));
        assert!(matches!(&e2e.environment[1], ServiceEnvOption::Simple(n) if n == "API_KEY"));
        assert!(
            matches!(&e2e.environment[2], ServiceEnvOption::WithValue(n, v) if n == "E2E_URL" && v == "${PUBLIC_URL}/api")
        );
        assert!(matches!(&e2e.secrets[0].mount, SecretMount::EnvVariable(v) if v == "PGPASSWORD"));

        let smoke = &suites[1];
        assert!(matches!(&smoke.deployment, SuiteDeployment::Named(n) if n == "local"));
        assert_eq!(smoke.deployment_name(), Some("local"));
        assert_eq!(smoke.working_dir, Path::new("/project").join("."));
        assert_eq!(smoke.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn a_suite_without_a_deployment_takes_the_only_one() {
        let suites = convert("suites:\n  e2e:\n    run: npm test\n").unwrap();
        assert!(matches!(suites[0].deployment, SuiteDeployment::Default));
        assert_eq!(suites[0].deployment_name(), None);
    }

    #[test]
    fn a_suite_needs_a_run_command() {
        let err = convert("suites:\n  e2e:\n    deployment: local\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Suite 'e2e' has no 'run' command"), "{err}");
        let err = convert("suites:\n  e2e:\n    run: '  '\n").unwrap_err().to_string();
        assert!(err.contains("no 'run' command"), "{err}");
    }

    #[test]
    fn an_abstract_inline_deployment_is_rejected() {
        let err = convert("suites:\n  e2e:\n    deployment:\n      abstract: true\n    run: x\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("abstract"), "{err}");
    }

    #[test]
    fn a_bad_timeout_is_rejected() {
        let err = convert("suites:\n  e2e:\n    timeout: soon\n    run: x\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("invalid timeout 'soon'"), "{err}");
    }

    #[test]
    fn an_empty_spec_is_rejected() {
        let err = convert("suites: {}\n").unwrap_err().to_string();
        assert!(err.contains("no suites"), "{err}");
    }
}
