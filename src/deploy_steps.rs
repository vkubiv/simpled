//! `simpled step ...`: the parts of a Swarm deploy that would otherwise be a lot
//! of generated shell. deploy.sh stays the readable outline (its order, its
//! phases, every `docker stack deploy`) and calls the simpled binary shipped next
//! to it for these. Each step reads a `plan.json` and talks to Docker through the
//! `docker` CLI, the same commands the shell used to run.
//!
//! A step that has to set variables for the rest of deploy.sh (object names, the
//! node to pin to) prints `export NAME='value'` lines on stdout, which deploy.sh
//! evaluates. Everything meant for a person goes to stderr.

use crate::deploy_plan::{interpolate, Plan, PlanJob};
use crate::secret_fetch::sh_quote;
use crate::swarm_objects::ObjectKind;
use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// The outcome of one `docker` command.
pub struct DockerOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `docker` with the given arguments. A trait so the steps can be tested
/// without a Docker daemon.
pub trait Docker {
    fn run(&self, args: &[&str]) -> Result<DockerOutput>;
}

pub struct DockerCli;

impl Docker for DockerCli {
    fn run(&self, args: &[&str]) -> Result<DockerOutput> {
        let output = Command::new("docker")
            .args(args)
            .output()
            .context("Failed to run docker. Is the Docker CLI installed and on PATH?")?;
        Ok(DockerOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

/// Runs a command that must succeed, and returns its stdout.
fn checked(docker: &dyn Docker, args: &[&str]) -> Result<String> {
    let output = docker.run(args)?;
    if !output.success {
        bail!("docker {} failed: {}", args.join(" "), output.stderr.trim());
    }
    Ok(output.stdout)
}

/// `export NAME='value'`, for deploy.sh to evaluate.
pub fn export_line(name: &str, value: &str) -> String {
    format!("export {}={}", name, sh_quote(value))
}

// ---------------------------------------------------------------- check-nodes

/// `multi_node: false`: the environment lives on the one node that accepts
/// tasks (`Ready` and `Active`). Fails unless there is exactly one, and returns
/// its ID for the `node.id` constraint every service carries. The node need not
/// be the one running the step: a drained manager with a single worker pins
/// everything to the worker.
pub fn check_nodes(docker: &dyn Docker) -> Result<String> {
    let listing = checked(
        docker,
        &[
            "node",
            "ls",
            "--format",
            "{{.ID}} {{.Hostname}} {{.Status}} {{.Availability}}",
        ],
    )?;
    let schedulable: Vec<(&str, &str)> = listing
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                [id, host, "Ready", "Active"] => Some((*id, *host)),
                _ => None,
            }
        })
        .collect();
    match schedulable.as_slice() {
        [] => bail!("No node in this swarm accepts tasks: every node is down or drained."),
        [(id, _)] => Ok(id.to_string()),
        many => {
            let hosts: Vec<&str> = many.iter().map(|(_, host)| *host).collect();
            bail!(
                "This swarm has {} nodes that accept tasks ({}), but the env spec sets multi_node: false.\n\
                 Set multi_node: true and bind the volumes (simpled docs reference --section volumes), \
                 or drain the extra nodes (docker node update --availability drain <node>).",
                many.len(),
                hosts.join(", ")
            )
        }
    }
}

// ---------------------------------------------------------------- objects

/// The Swarm name of an object created from `content`: its base and the first 12
/// hex digits of the content's sha256.
pub fn object_name(base: &str, content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    format!("{}_{}", base, &hex[..12])
}

fn kind_command(kind: ObjectKind) -> &'static str {
    kind.command()
}

/// Creates every object of the plan that does not exist yet, under its
/// content-addressed name, and returns `(variable, name)` for each. Swarm objects
/// cannot be changed, so new content is a new object.
pub fn sync_objects(docker: &dyn Docker, plan: &Plan, deploy_dir: &Path) -> Result<Vec<(String, String)>> {
    let mut exports = Vec::new();
    for object in &plan.objects {
        let path = deploy_dir.join(&object.file);
        let content = std::fs::read(&path).with_context(|| {
            format!(
                "Cannot read {}, the file {} {} is created from",
                path.display(),
                kind_command(object.kind),
                object.base
            )
        })?;
        let name = object_name(&object.base, &content);
        let kind = kind_command(object.kind);
        if !docker.run(&[kind, "inspect", &name])?.success {
            let file = path.to_string_lossy();
            checked(
                docker,
                &[
                    kind,
                    "create",
                    "--label",
                    &plan.owner_label,
                    "--label",
                    &format!("simpled.object={}", object.base),
                    &name,
                    &file,
                ],
            )?;
            eprintln!("Created {} {}", kind, name);
        }
        exports.push((object.var.clone(), name));
    }
    Ok(exports)
}

/// Removes older objects: of each base name, the two newest stay, the current
/// one and the previous one, which a `docker service rollback` still mounts.
/// Docker refuses to remove an object a service still uses, so an older one in
/// use stays too. Only objects carrying the plan's owner label are touched.
pub fn prune_objects(docker: &dyn Docker, plan: &Plan) -> Result<()> {
    for object in &plan.objects {
        let kind = kind_command(object.kind);
        let ids = checked(
            docker,
            &[
                kind,
                "ls",
                "-q",
                "--filter",
                &format!("label={}", plan.owner_label),
                "--filter",
                &format!("label=simpled.object={}", object.base),
            ],
        )?;
        let ids: Vec<&str> = ids.lines().filter(|l| !l.trim().is_empty()).collect();
        if ids.len() <= 2 {
            continue;
        }
        // `ls` only shows relative ages ("4 minutes ago"), so the creation time
        // comes from `inspect`, as a fixed-width UTC timestamp that sorts as text.
        let mut args = vec![
            kind,
            "inspect",
            "--format",
            "{{.CreatedAt.UTC.Format \"2006-01-02T15:04:05.000000000Z\"}} {{.Spec.Name}}",
        ];
        args.extend(ids.iter().copied());
        let listing = checked(docker, &args)?;
        let mut created: Vec<(&str, &str)> = listing.lines().filter_map(|l| l.split_once(' ')).collect();
        created.sort_by(|a, b| b.0.cmp(a.0));
        for (_, name) in created.into_iter().skip(2) {
            if docker.run(&[kind, "rm", name])?.success {
                eprintln!("Removed {} {}", kind, name);
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- run-job

/// Runs a job as a `replicated-job` service and waits for it to finish.
///
/// Swarm's run-to-completion mode exists only in the CLI and API, not in stack
/// files, so a job is its own service. It is created detached and its task is
/// polled for a terminal state: `docker service create --detach=false` waits
/// forever on a task that failed, so the timeout would never get to run. The
/// service is removed afterwards, whatever the result.
pub fn run_job(
    docker: &dyn Docker,
    job: &PlanJob,
    lookup: &dyn Fn(&str) -> Option<String>,
    timeout: Duration,
    poll: Duration,
) -> Result<()> {
    eprintln!("Running job {}...", job.service);
    let mut args: Vec<String> = vec![
        "service".into(),
        "create".into(),
        "--name".into(),
        job.service.clone(),
        "--mode".into(),
        "replicated-job".into(),
        "--restart-condition".into(),
        "none".into(),
        "--detach".into(),
        "--quiet".into(),
        "--with-registry-auth".into(),
    ];
    for template in &job.args {
        args.push(interpolate(template, lookup).with_context(|| format!("Job {}", job.name))?);
    }

    // A service left over from an earlier deploy would make the name clash.
    docker.run(&["service", "rm", &job.service])?;

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let created = docker.run(&arg_refs)?;
    if !created.success {
        bail!("Job {} could not be created: {}", job.service, created.stderr.trim());
    }

    let started = Instant::now();
    let state = loop {
        let state = checked(
            docker,
            &["service", "ps", &job.service, "--format", "{{.CurrentState}}"],
        )?
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
        let terminal = ["Complete", "Failed", "Rejected", "Orphaned", "Shutdown"]
            .iter()
            .any(|t| state.starts_with(t));
        if terminal || started.elapsed() >= timeout {
            break state;
        }
        std::thread::sleep(poll);
    };

    let completed = state.starts_with("Complete");
    let tail = if completed { "50" } else { "200" };
    let logs = docker.run(&["service", "logs", "--tail", tail, &job.service])?;
    let removed = docker.run(&["service", "rm", &job.service]);
    if completed {
        print!("{}", logs.stdout);
        eprint!("{}", logs.stderr);
        removed?;
        eprintln!("Job {} completed.", job.service);
        return Ok(());
    }
    eprint!("{}{}", logs.stdout, logs.stderr);
    let state = if state.is_empty() { "unknown".to_string() } else { state };
    Err(anyhow!(
        "Job {} did not complete (state: {}){}",
        job.service,
        state,
        if started.elapsed() >= timeout {
            format!(", gave up after {}s; JOB_TIMEOUT raises the limit", timeout.as_secs())
        } else {
            String::new()
        }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy_plan::PlanObject;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Answers each command from a table, by its first matching prefix, and
    /// records every call.
    struct FakeDocker {
        answers: Vec<(String, bool, String)>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeDocker {
        fn new(answers: &[(&str, bool, &str)]) -> Self {
            FakeDocker {
                answers: answers
                    .iter()
                    .map(|(p, ok, out)| (p.to_string(), *ok, out.to_string()))
                    .collect(),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Docker for FakeDocker {
        fn run(&self, args: &[&str]) -> Result<DockerOutput> {
            let call = args.join(" ");
            self.calls.borrow_mut().push(call.clone());
            let (success, stdout) = self
                .answers
                .iter()
                .find(|(prefix, _, _)| call.starts_with(prefix.as_str()))
                .map(|(_, ok, out)| (*ok, out.clone()))
                .unwrap_or((true, String::new()));
            Ok(DockerOutput {
                success,
                stdout,
                stderr: if success { String::new() } else { "boom".to_string() },
            })
        }
    }

    fn plan(objects: Vec<PlanObject>) -> Plan {
        Plan {
            simpled_version: "2.0.0".to_string(),
            owner_label: "simpled.deployment=prod".to_string(),
            objects,
            jobs: vec![],
        }
    }

    fn object(kind: ObjectKind, base: &str, file: &str, var: &str) -> PlanObject {
        PlanObject {
            kind,
            base: base.to_string(),
            file: file.to_string(),
            var: var.to_string(),
        }
    }

    #[test]
    fn the_one_schedulable_node_is_pinned() {
        let docker = FakeDocker::new(&[(
            "node ls",
            true,
            "m1 manager Ready Drain\nw1 worker Ready Active\nw2 old Down Active\n",
        )]);
        assert_eq!(check_nodes(&docker).unwrap(), "w1");

        let two = FakeDocker::new(&[("node ls", true, "m1 manager Ready Active\nw1 worker Ready Active\n")]);
        let err = check_nodes(&two).unwrap_err().to_string();
        assert!(err.contains("2 nodes that accept tasks (manager, worker)"), "{}", err);

        let none = FakeDocker::new(&[("node ls", true, "m1 manager Ready Drain\n")]);
        assert!(check_nodes(&none)
            .unwrap_err()
            .to_string()
            .contains("No node in this swarm accepts tasks"));
    }

    #[test]
    fn objects_are_created_once_under_their_content_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("prod/secrets")).unwrap();
        std::fs::write(dir.path().join("prod/secrets/key"), "abc").unwrap();
        std::fs::create_dir_all(dir.path().join("prod/configs/data")).unwrap();
        std::fs::write(dir.path().join("prod/configs/data/a.json"), "{}").unwrap();
        let plan = plan(vec![
            object(ObjectKind::Secret, "prod_key", "prod/secrets/key", "SIMPLED_OBJECT_1"),
            object(
                ObjectKind::Config,
                "prod_data_a.json",
                "prod/configs/data/a.json",
                "SIMPLED_OBJECT_2",
            ),
        ]);
        // sha256("abc") starts ba7816bf8f01.
        let key = "prod_key_ba7816bf8f01";
        let docker = FakeDocker::new(&[
            (&format!("secret inspect {}", key), false, ""),
            ("config inspect", true, "[{}]"),
        ]);

        let exports = sync_objects(&docker, &plan, dir.path()).unwrap();
        assert_eq!(exports[0], ("SIMPLED_OBJECT_1".to_string(), key.to_string()));
        assert!(exports[1].1.starts_with("prod_data_a.json_"));
        let calls = docker.calls();
        let create = calls
            .iter()
            .find(|c| c.starts_with("secret create"))
            .expect("secret created");
        assert!(
            create.contains("--label simpled.deployment=prod --label simpled.object=prod_key prod_key_ba7816bf8f01")
        );
        // The config already exists, so it is left alone.
        assert!(!calls.iter().any(|c| c.starts_with("config create")), "{:?}", calls);
    }

    #[test]
    fn a_missing_object_file_names_the_object() {
        let dir = tempfile::tempdir().unwrap();
        let plan = plan(vec![object(ObjectKind::Secret, "prod_key", "prod/secrets/key", "V")]);
        let err = format!(
            "{:#}",
            sync_objects(&FakeDocker::new(&[]), &plan, dir.path()).unwrap_err()
        );
        assert!(err.contains("the file secret prod_key is created from"), "{}", err);
    }

    #[test]
    fn pruning_keeps_the_two_newest_of_each_object() {
        let plan = plan(vec![object(ObjectKind::Config, "prod_cfg", "f", "V")]);
        let docker = FakeDocker::new(&[
            ("config ls", true, "id1\nid2\nid3\nid4\n"),
            (
                "config inspect",
                true,
                // Out of order on purpose; 10 minutes old must not look newer than 9.
                "2026-11-04T15:10:00.000000000Z prod_cfg_b\n\
                 2026-11-04T15:20:00.000000000Z prod_cfg_d\n\
                 2026-11-04T15:05:00.000000000Z prod_cfg_a\n\
                 2026-11-04T15:15:00.000000000Z prod_cfg_c\n",
            ),
            ("config rm prod_cfg_a", false, ""),
        ]);
        prune_objects(&docker, &plan).unwrap();
        let removed: Vec<String> = docker
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("config rm"))
            .collect();
        assert_eq!(removed, vec!["config rm prod_cfg_b", "config rm prod_cfg_a"]);
        assert!(
            docker.calls()[0].contains("--filter label=simpled.deployment=prod --filter label=simpled.object=prod_cfg")
        );
    }

    fn job() -> PlanJob {
        PlanJob {
            name: "migrate".to_string(),
            service: "prod_migrate".to_string(),
            args: vec![
                "--config".to_string(),
                "source=${SIMPLED_OBJECT_1},target=/data/a.json,mode=444".to_string(),
                "-e".to_string(),
                "PASSWORD=pa$$word".to_string(),
                "--mount".to_string(),
                "type=bind,source=${DEPLOY_DIR}/prod/data,target=/data".to_string(),
                "registry.example.com/migrate:1.0.0".to_string(),
            ],
            timeout_seconds: 600,
        }
    }

    fn lookup() -> impl Fn(&str) -> Option<String> {
        let env: HashMap<&str, &str> = [
            ("SIMPLED_OBJECT_1", "prod_data_a.json_123"),
            ("DEPLOY_DIR", "/srv/deploy"),
        ]
        .into_iter()
        .collect();
        move |name| env.get(name).map(|v| v.to_string())
    }

    #[test]
    fn a_job_runs_with_its_templates_expanded_and_is_removed() {
        let docker = FakeDocker::new(&[("service ps", true, "Complete 1 second ago\n")]);
        run_job(&docker, &job(), &lookup(), Duration::from_secs(5), Duration::ZERO).unwrap();
        let calls = docker.calls();
        let create = calls.iter().find(|c| c.starts_with("service create")).unwrap();
        assert!(create.starts_with(
            "service create --name prod_migrate --mode replicated-job --restart-condition none --detach --quiet --with-registry-auth"
        ));
        assert!(
            create.contains("--config source=prod_data_a.json_123,target=/data/a.json,mode=444"),
            "{}",
            create
        );
        assert!(create.contains("-e PASSWORD=pa$word"), "{}", create);
        assert!(create.contains("source=/srv/deploy/prod/data"), "{}", create);
        assert_eq!(calls.first().unwrap(), "service rm prod_migrate");
        assert_eq!(calls.last().unwrap(), "service rm prod_migrate");
    }

    #[test]
    fn a_failed_job_fails_the_step_and_is_still_removed() {
        let docker = FakeDocker::new(&[("service ps", true, "Failed 2 seconds ago\n")]);
        let err = run_job(&docker, &job(), &lookup(), Duration::from_secs(5), Duration::ZERO).unwrap_err();
        assert!(
            err.to_string()
                .contains("did not complete (state: Failed 2 seconds ago)"),
            "{}",
            err
        );
        assert!(docker
            .calls()
            .iter()
            .any(|c| c == "service logs --tail 200 prod_migrate"));
        assert_eq!(docker.calls().last().unwrap(), "service rm prod_migrate");
    }

    #[test]
    fn a_job_that_never_finishes_times_out() {
        let docker = FakeDocker::new(&[("service ps", true, "Running 3 seconds ago\n")]);
        let err = run_job(&docker, &job(), &lookup(), Duration::ZERO, Duration::ZERO).unwrap_err();
        assert!(
            err.to_string()
                .contains("gave up after 0s; JOB_TIMEOUT raises the limit"),
            "{}",
            err
        );
    }

    #[test]
    fn a_job_needing_an_unset_variable_is_not_created() {
        let docker = FakeDocker::new(&[]);
        let err = format!(
            "{:#}",
            run_job(&docker, &job(), &|_: &str| None, Duration::ZERO, Duration::ZERO).unwrap_err()
        );
        assert!(err.contains("SIMPLED_OBJECT_1 is not set"), "{}", err);
        assert!(docker.calls().is_empty());
    }
}
