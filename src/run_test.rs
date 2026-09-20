//! `simpled test`: bring a local deployment up, run a suite against it, tear it
//! down, and exit with the suite's status.
//!
//! The stack lives in its own compose project and directory (`test_env/`, named
//! `<app>_test`) with Docker-managed volumes, so a run starts from empty data
//! and `down --volumes` leaves nothing behind, whatever the developer's own
//! `local_env` stack is doing.

use crate::docker_compose::write_host_env;
use crate::local_ingress::{self, IngressHandle};
use crate::resolved_spec::{EnvironmentResolvedSpec, ServiceResolvedSpec};
use crate::run_local::{write_compose, ComposeTarget};
use crate::spec::{DeploymentEnvType, EnvVariable, ServiceEnvOption, ServiceSecret};
use crate::test_spec::{self, SuiteDeployment, TestSuite};
use crate::{resolver, spec_loader, transform, validator};
use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

pub struct TestOptions {
    /// Leave the stack running after the suite, to debug a failure.
    pub keep: bool,
    /// Run the suite against whatever is already up: no compose, no gateway, no
    /// readiness wait.
    pub no_up: bool,
    /// Print the services' logs after every suite, not only a failing one.
    pub logs: bool,
}

/// Set by the Ctrl-C handler. The running child gets the signal too and exits on
/// its own; this process stays alive to bring the stack down.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
static INSTALL_HANDLER: Once = Once::new();

const INTERRUPTED_EXIT_CODE: i32 = 130;
const LOG_TAIL_LINES: &str = "200";

/// Runs the named suite, or every suite in name order, and returns the exit code
/// to end the process with: the first failing suite's, or 0.
pub fn run(root: &Path, suite_name: Option<&str>, options: &TestOptions) -> Result<i32> {
    let suites = test_spec::load_test_spec(root)?;
    let selected: Vec<&TestSuite> = match suite_name {
        Some(name) => vec![suites.iter().find(|s| s.name == name).ok_or_else(|| {
            let names: Vec<&str> = suites.iter().map(|s| s.name.as_str()).collect();
            anyhow!(
                "Suite '{}' not found in testspec.yaml. Available suites: {}",
                name,
                names.join(", ")
            )
        })?],
        None => suites.iter().collect(),
    };

    let mut first_failure = 0;
    for suite in selected {
        let code = run_suite(root, suite, options)?;
        if code == 0 {
            println!("Suite '{}' passed", suite.name);
        } else {
            println!("Suite '{}' failed with exit code {}", suite.name, code);
            if first_failure == 0 {
                first_failure = code;
            }
        }
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Ok(INTERRUPTED_EXIT_CODE);
        }
    }
    Ok(first_failure)
}

fn run_suite(root: &Path, suite: &TestSuite, options: &TestOptions) -> Result<i32> {
    println!("Running suite '{}'", suite.name);

    let resolved = resolve_suite_deployment(root, suite)?;
    let deployment_name = resolved.current_deployment.name.clone();

    if !suite.working_dir.is_dir() {
        bail!(
            "Suite '{}' has working_dir {:?}, which is not a directory",
            suite.name,
            suite.working_dir
        );
    }
    let env_vars = resolve_suite_environment(suite, &resolved)?;
    let secrets = resolve_suite_secrets(suite, &resolved)?;
    let env_vars = write_host_env(
        &suite.working_dir,
        &env_vars,
        &secrets,
        &resolved,
        &format!("suite '{}'", suite.name),
    )?;

    if options.no_up {
        return run_command(suite, &env_vars);
    }

    let target = ComposeTarget::test(&resolved);
    write_compose(&resolved, &target, |s| !s.excluded)?;

    // Bound before compose starts, so a port held by a forgotten `local run`
    // fails here with nothing to clean up.
    let ingress = local_ingress::run(resolved.ingress.clone(), &deployment_name, "127.0.0.1")?;
    install_interrupt_handler();

    let stack = Stack {
        target: &target,
        ingress: Some(ingress),
    };

    let deadline = Instant::now() + suite.timeout;
    let outcome = stack
        .up(&resolved, deadline)
        .and_then(|()| wait_for_urls(&suite.wait_for, deadline))
        .and_then(|()| run_command(suite, &env_vars));

    let failed = !matches!(outcome, Ok(0));
    if options.logs || failed {
        stack.logs();
    }
    if options.keep {
        println!(
            "Stack left running (--keep). Stop it with: docker compose -f {}/docker-compose.yaml down --volumes",
            target.dir.display()
        );
        stack.stop_ingress();
    } else {
        stack.down();
    }

    outcome
}

/// The env spec with the suite's deployment selected: a named one, the only one,
/// or the suite's own block registered under the suite's name.
fn resolve_suite_deployment(root: &Path, suite: &TestSuite) -> Result<EnvironmentResolvedSpec> {
    let mut env_yaml = spec_loader::load_env_spec_yaml(root)?;

    if let SuiteDeployment::Inline(dep) = &suite.deployment {
        if env_yaml.deployments.contains_key(&suite.name) {
            bail!(
                "Suite '{}' declares its own deployment, but the env spec already has a deployment named '{}'. \
                 Rename the suite or reference that deployment by name.",
                suite.name,
                suite.name
            );
        }
        env_yaml.deployments.insert(suite.name.clone(), (**dep).clone());
    }

    let selected = match &suite.deployment {
        SuiteDeployment::Default => {
            let mut concrete: Vec<&String> = env_yaml
                .deployments
                .iter()
                .filter(|(_, d)| d.is_abstract != Some(true))
                .map(|(name, _)| name)
                .collect();
            concrete.sort();
            match concrete.as_slice() {
                [only] => Some(only.to_string()),
                [] => bail!("The env spec defines no deployment for suite '{}' to run", suite.name),
                many => bail!(
                    "Suite '{}' does not name a deployment and the env spec defines several ({}). \
                     Set 'deployment:' on the suite.",
                    suite.name,
                    many.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ),
            }
        }
        _ => suite.deployment_name().map(str::to_string),
    };

    let env_spec = transform::convert_env_spec(env_yaml, root, selected.as_deref())
        .with_context(|| format!("Failed to process env spec for suite '{}'", suite.name))?;
    if env_spec.env_type != DeploymentEnvType::Local {
        bail!(
            "simpled test runs against a local environment; {:?} has no localenv.yaml",
            root
        );
    }
    let deployment_name = selected
        .or_else(|| env_spec.deployments.first().map(|d| d.name.clone()))
        .ok_or_else(|| anyhow!("No deployment to run for suite '{}'", suite.name))?;

    let app_spec = spec_loader::load_app_spec_from_dir(root, Some(&env_spec))?;
    validator::validate(&env_spec, &app_spec, &deployment_name).context("Validation failed")?;
    resolver::resolve(&env_spec, &app_spec, &deployment_name).context("Resolution failed")
}

/// The suite's variables, resolved against the deployment as a process on this
/// machine sees it. A forwarded name that the deployment does not define is an
/// error, as is a `${VAR}` reference to one.
fn resolve_suite_environment(suite: &TestSuite, resolved: &EnvironmentResolvedSpec) -> Result<Vec<EnvVariable>> {
    let host_env = &resolved.current_deployment.host_environment;
    let deployment = &resolved.current_deployment.name;
    let mut vars: Vec<EnvVariable> = Vec::new();
    let mut set = |var: EnvVariable| match vars.iter_mut().find(|v| v.name == var.name) {
        Some(existing) => existing.value = var.value,
        None => vars.push(var),
    };

    for option in &suite.environment {
        match option {
            ServiceEnvOption::All => host_env.iter().cloned().for_each(&mut set),
            ServiceEnvOption::Simple(name) => {
                let value = host_env.iter().find(|v| &v.name == name).ok_or_else(|| {
                    anyhow!(
                        "Suite '{}' forwards {}, which deployment '{}' does not define",
                        suite.name,
                        name,
                        deployment
                    )
                })?;
                set(value.clone());
            }
            ServiceEnvOption::WithValue(name, raw) => {
                let value = resolver::resolve_variable_in_string(raw, host_env).with_context(|| {
                    format!(
                        "Suite '{}' sets {} from a variable deployment '{}' does not define",
                        suite.name, name, deployment
                    )
                })?;
                set(EnvVariable {
                    name: name.clone(),
                    value,
                });
            }
        }
    }
    Ok(vars)
}

/// The suite's secrets under the names the resolved deployment carries them.
fn resolve_suite_secrets(suite: &TestSuite, resolved: &EnvironmentResolvedSpec) -> Result<Vec<ServiceSecret>> {
    let deployment = &resolved.current_deployment;
    suite
        .secrets
        .iter()
        .map(|secret| {
            let full_name = format!("{}-{}", deployment.application_name, secret.name);
            if !deployment.secrets.iter().any(|s| s.name == full_name) {
                bail!(
                    "Suite '{}' uses secret '{}', which deployment '{}' does not provide",
                    suite.name,
                    secret.name,
                    deployment.name
                );
            }
            Ok(ServiceSecret {
                name: full_name,
                mount: secret.mount.clone(),
            })
        })
        .collect()
}

/// Runs the suite's command through the platform shell, in its working
/// directory, with its variables in the environment as well as in `.env`.
fn run_command(suite: &TestSuite, env_vars: &[EnvVariable]) -> Result<i32> {
    println!("Running: {}", suite.run);
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", &suite.run]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", &suite.run]);
        c
    };
    command.current_dir(&suite.working_dir);
    for var in env_vars {
        command.env(&var.name, &var.value);
    }
    let status = command
        .status()
        .with_context(|| format!("Failed to run '{}' for suite '{}'", suite.run, suite.name))?;
    if INTERRUPTED.load(Ordering::SeqCst) {
        return Ok(INTERRUPTED_EXIT_CODE);
    }
    Ok(status.code().unwrap_or(1))
}

fn install_interrupt_handler() {
    INSTALL_HANDLER.call_once(|| {
        if let Err(e) = ctrlc::set_handler(|| {
            INTERRUPTED.store(true, Ordering::SeqCst);
            eprintln!("Interrupted; bringing the test stack down...");
        }) {
            eprintln!(
                "Warning: could not install a Ctrl-C handler ({}); an interrupted run leaves its stack up",
                e
            );
        }
    });
}

/// Polls every URL until each has answered, or `deadline` passes. Any HTTP
/// response below 500 counts: the point is that something is listening and
/// serving, not what it says about that path.
fn wait_for_urls(urls: &[String], deadline: Instant) -> Result<()> {
    if urls.is_empty() {
        return Ok(());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("Failed to build an HTTP client")?;
    for url in urls {
        println!("Waiting for {}...", url);
        loop {
            if INTERRUPTED.load(Ordering::SeqCst) {
                bail!("Interrupted while waiting for {}", url);
            }
            match client.get(url).send() {
                Ok(response) if response.status().as_u16() < 500 => break,
                Ok(response) => {
                    if Instant::now() >= deadline {
                        bail!("{} still answers {} after the suite's timeout", url, response.status());
                    }
                }
                Err(e) => {
                    if Instant::now() >= deadline {
                        bail!("{} did not answer within the suite's timeout: {}", url, e);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    Ok(())
}

fn check_not_interrupted(doing: &str) -> Result<()> {
    if INTERRUPTED.load(Ordering::SeqCst) {
        bail!("Interrupted while {}", doing);
    }
    Ok(())
}

/// Whole seconds left before `deadline`, or an error once it has passed.
fn remaining_secs(deadline: Instant) -> Result<u64> {
    let left = deadline.saturating_duration_since(Instant::now()).as_secs();
    if left == 0 {
        bail!("The suite's timeout ran out before the stack was up");
    }
    Ok(left)
}

/// The compose project of one suite run, and the gateway in front of it.
struct Stack<'a> {
    target: &'a ComposeTarget,
    ingress: Option<IngressHandle>,
}

impl Stack<'_> {
    fn compose(&self) -> Command {
        let mut command = Command::new("docker");
        command.current_dir(&self.target.dir).arg("compose");
        command
    }

    /// Starts the stack in the phases the deploy scripts use: what the jobs
    /// need, then each job to completion, then everything else. `compose up
    /// --wait` alone would not do: it reports a job that exited 0 as a failure
    /// and says nothing about one that exited 1, while `compose wait` returns
    /// the job's own status.
    fn up(&self, spec: &EnvironmentResolvedSpec, deadline: Instant) -> Result<()> {
        let deployment = &spec.current_deployment;
        let included = |services: Vec<&ServiceResolvedSpec>| -> Vec<String> {
            services
                .into_iter()
                .filter(|s| !s.excluded)
                .map(|s| s.full_name.clone())
                .collect()
        };
        let prerequisites = included(deployment.job_prerequisites());
        let jobs = included(deployment.jobs_in_order());
        let rest: Vec<String> = included(deployment.long_running_services())
            .into_iter()
            .filter(|name| !prerequisites.contains(name))
            .collect();

        self.up_and_wait(&prerequisites, deadline)?;
        for job in &jobs {
            self.run_job(job, deadline)?;
        }
        self.up_and_wait(&rest, deadline)
    }

    fn up_and_wait(&self, services: &[String], deadline: Instant) -> Result<()> {
        if services.is_empty() {
            return Ok(());
        }
        println!("Starting {}...", services.join(", "));
        let status = self
            .compose()
            .args(["up", "--detach", "--remove-orphans", "--wait", "--wait-timeout"])
            .arg(remaining_secs(deadline)?.to_string())
            .args(services)
            .status()
            .context("Failed to run docker compose")?;
        check_not_interrupted("starting the stack")?;
        if !status.success() {
            bail!("docker compose up failed for {}", services.join(", "));
        }
        Ok(())
    }

    fn run_job(&self, job: &str, deadline: Instant) -> Result<()> {
        println!("Running job {}...", job);
        let status = self
            .compose()
            .args(["up", "--detach", job])
            .status()
            .context("Failed to run docker compose")?;
        check_not_interrupted("starting a job")?;
        if !status.success() {
            bail!("docker compose up failed for job {}", job);
        }

        // Polled rather than `compose wait`: a quick job has already exited by
        // the time `up` returns, and `wait` then finds nothing to wait on.
        loop {
            if let Some(code) = self.job_exit_code(job)? {
                if code != 0 {
                    bail!("Job {} failed with exit code {}", job, code);
                }
                return Ok(());
            }
            check_not_interrupted("waiting for a job")?;
            if Instant::now() >= deadline {
                bail!("Job {} did not finish within the suite's timeout", job);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// The job's exit code once its container has stopped, `None` while it is
    /// still running (or not yet listed).
    fn job_exit_code(&self, job: &str) -> Result<Option<i64>> {
        let output = self
            .compose()
            .args(["ps", "--all", "--format", "json", job])
            .output()
            .context("Failed to run docker compose ps")?;
        if !output.status.success() {
            bail!(
                "docker compose ps failed for job {}: {}",
                job,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        // One JSON object per line; JSON is YAML, so no extra parser is needed.
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let container: serde_yaml::Value = serde_yaml::from_str(line)
                .with_context(|| format!("Unexpected docker compose ps output for job {}: {}", job, line))?;
            let state = container["State"].as_str().unwrap_or_default();
            if matches!(state, "exited" | "dead") {
                return Ok(Some(container["ExitCode"].as_i64().unwrap_or(1)));
            }
        }
        Ok(None)
    }

    fn logs(&self) {
        println!("--- service logs (last {} lines each) ---", LOG_TAIL_LINES);
        let _ = self
            .compose()
            .args(["logs", "--no-color", "--tail", LOG_TAIL_LINES])
            .status();
        println!("--- end of service logs ---");
    }

    fn down(mut self) {
        println!("Running docker compose down...");
        let _ = self.compose().args(["down", "--remove-orphans", "--volumes"]).status();
        if let Some(ingress) = self.ingress.take() {
            ingress.stop();
        }
    }

    fn stop_ingress(mut self) {
        if let Some(ingress) = self.ingress.take() {
            ingress.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::{SecretResolvedSpec, SecretResolvedValue};
    use crate::spec::SecretMount;
    use crate::test_support::{resolved_deployment, resolved_env};

    fn var(name: &str, value: &str) -> EnvVariable {
        EnvVariable {
            name: name.to_string(),
            value: value.to_string(),
        }
    }

    fn make_suite(environment: Vec<ServiceEnvOption>, secrets: Vec<ServiceSecret>) -> TestSuite {
        TestSuite {
            name: "e2e".to_string(),
            deployment: SuiteDeployment::Named("local".to_string()),
            working_dir: Path::new(".").to_path_buf(),
            environment,
            secrets,
            wait_for: vec![],
            timeout: Duration::from_secs(1),
            run: "true".to_string(),
        }
    }

    fn env() -> EnvironmentResolvedSpec {
        let mut deployment = resolved_deployment(vec![]);
        deployment.name = "local".to_string();
        deployment.host_environment = vec![var("PUBLIC_URL", "http://localhost:8080"), var("API_KEY", "k")];
        deployment.secrets = vec![SecretResolvedSpec {
            name: "shop-db_password".to_string(),
            value: SecretResolvedValue::Literal("pw".to_string()),
        }];
        resolved_env(DeploymentEnvType::Local, deployment)
    }

    #[test]
    fn suite_variables_forward_substitute_and_override() {
        let suite = make_suite(
            vec![
                ServiceEnvOption::All,
                ServiceEnvOption::WithValue("E2E_URL".to_string(), "${PUBLIC_URL}/api".to_string()),
                ServiceEnvOption::WithValue("API_KEY".to_string(), "override".to_string()),
            ],
            vec![],
        );
        let vars = resolve_suite_environment(&suite, &env()).unwrap();
        let get = |n: &str| vars.iter().find(|v| v.name == n).map(|v| v.value.as_str());
        assert_eq!(get("PUBLIC_URL"), Some("http://localhost:8080"));
        assert_eq!(get("E2E_URL"), Some("http://localhost:8080/api"));
        // A later entry replaces an earlier one in place.
        assert_eq!(get("API_KEY"), Some("override"));
        assert_eq!(vars.len(), 3);
    }

    #[test]
    fn forwarding_an_unknown_variable_names_the_suite_and_deployment() {
        let suite = make_suite(vec![ServiceEnvOption::Simple("NOPE".to_string())], vec![]);
        let err = resolve_suite_environment(&suite, &env()).unwrap_err().to_string();
        assert_eq!(
            err,
            "Suite 'e2e' forwards NOPE, which deployment 'local' does not define"
        );

        let suite = make_suite(
            vec![ServiceEnvOption::WithValue("X".to_string(), "${NOPE}".to_string())],
            vec![],
        );
        let err = format!("{:#}", resolve_suite_environment(&suite, &env()).unwrap_err());
        assert!(
            err.contains("Suite 'e2e' sets X from a variable deployment 'local' does not define"),
            "{err}"
        );
        assert!(err.contains("Undefined variable: NOPE"), "{err}");
    }

    #[test]
    fn suite_secrets_take_the_deployments_prefixed_names() {
        let suite = make_suite(
            vec![],
            vec![ServiceSecret {
                name: "db_password".to_string(),
                mount: SecretMount::EnvVariable("PGPASSWORD".to_string()),
            }],
        );
        let secrets = resolve_suite_secrets(&suite, &env()).unwrap();
        assert_eq!(secrets[0].name, "shop-db_password");
        assert!(matches!(&secrets[0].mount, SecretMount::EnvVariable(v) if v == "PGPASSWORD"));

        let suite = make_suite(
            vec![],
            vec![ServiceSecret {
                name: "api_key".to_string(),
                mount: SecretMount::FilePath("/secrets/api_key".to_string()),
            }],
        );
        let err = resolve_suite_secrets(&suite, &env()).unwrap_err().to_string();
        assert_eq!(
            err,
            "Suite 'e2e' uses secret 'api_key', which deployment 'local' does not provide"
        );
    }
}
