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

    /// `run` with `input` on stdin, for content that should not touch the disk.
    fn run_input(&self, args: &[&str], _input: &[u8]) -> Result<DockerOutput> {
        self.run(args)
    }
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

    fn run_input(&self, args: &[&str], input: &[u8]) -> Result<DockerOutput> {
        use std::io::Write;
        let mut child = Command::new("docker")
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("Failed to run docker. Is the Docker CLI installed and on PATH?")?;
        child.stdin.take().context("docker has no stdin")?.write_all(input)?;
        let output = child.wait_with_output()?;
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

// ---------------------------------------------------------------- tls-secrets

/// The secret each of the gateway's domains is served with,
/// `gateway_tls::current`, the same one the controller rolls the gateway onto. A
/// domain with no secret gets a self-signed placeholder, which the controller
/// replaces once it has issued the real one. Exports the names for the
/// gateway's stack.
pub fn tls_secrets(docker: &dyn Docker, plan: &Plan) -> Result<Vec<(String, String)>> {
    use crate::gateway_tls::{labels, CertSecret, ISSUER_SELF_SIGNED};
    let mut exports = Vec::new();
    for tls in &plan.tls {
        let ids = checked(
            docker,
            &[
                "secret",
                "ls",
                "-q",
                "--filter",
                &format!("label={}={}", labels::DOMAIN, tls.domain),
            ],
        )?;
        let ids: Vec<&str> = ids.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let mut found = Vec::new();
        if !ids.is_empty() {
            let format = format!(
                "{{{{.ID}}}}|{{{{.Spec.Name}}}}|{{{{index .Spec.Labels \"{}\"}}}}|{{{{index .Spec.Labels \"{}\"}}}}|{{{{.Version.Index}}}}",
                labels::ISSUER,
                labels::NOT_AFTER
            );
            let mut args = vec!["secret", "inspect", "--format", format.as_str()];
            args.extend(ids.iter().copied());
            for line in checked(docker, &args)?.lines() {
                let parts: Vec<&str> = line.trim().split('|').collect();
                if let [id, name, issuer, not_after, index] = parts.as_slice() {
                    found.push(CertSecret::from_labels(
                        id.to_string(),
                        name.to_string(),
                        Some(*issuer),
                        Some(*not_after),
                        index.parse().unwrap_or_default(),
                    ));
                }
            }
        }
        let name = match crate::gateway_tls::current(&found) {
            Some(secret) => secret.name.clone(),
            None => {
                let pem = crate::gateway_tls::self_signed(&tls.domain)?;
                let name = object_name(&crate::gateway_tls::secret_base(&tls.domain), pem.as_bytes());
                let output = docker.run_input(
                    &secret_create_args(&tls.domain, ISSUER_SELF_SIGNED, 0, &name)
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    pem.as_bytes(),
                )?;
                if !output.success {
                    bail!(
                        "Cannot create the placeholder certificate of {}: {}",
                        tls.domain,
                        output.stderr.trim()
                    );
                }
                eprintln!(
                    "Created a self-signed placeholder for {}, until the controller has issued its certificate",
                    tls.domain
                );
                name
            }
        };
        exports.push((tls.var.clone(), name));
    }
    Ok(exports)
}

/// `docker secret create` of a certificate secret, labelled as the controller
/// reads it, from stdin.
fn secret_create_args(domain: &str, issuer: &str, not_after: u64, name: &str) -> Vec<String> {
    let mut args = vec!["secret".to_string(), "create".to_string()];
    args.extend(crate::gateway_tls::secret_label_args(domain, issuer, not_after));
    args.push(name.to_string());
    args.push("-".to_string());
    args
}

// ---------------------------------------------------------------- tls import

/// What an import of an `acme.json` did, per certificate name.
#[derive(Debug, Default, PartialEq)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub already_there: Vec<String>,
    /// Name and why: expired, a wildcard, or not one of the gateway's domains.
    pub skipped: Vec<(String, String)>,
}

/// Turns the certificates in Traefik's `acme.json` (simpled 2.0.x kept it in
/// `letsencrypt/`) into the gateway's certificate secrets, so an upgrade issues
/// nothing new. Each name a certificate covers gets its own secret, like the
/// ones the controller issues. `only` restricts the import to these domains.
/// Importing twice creates nothing new: the names are content-addressed.
pub fn import_acme_json(
    docker: &dyn Docker,
    acme_json: &str,
    only: Option<&[String]>,
    now: u64,
) -> Result<ImportReport> {
    use crate::gateway_tls::ISSUER_ACME;
    use base64::Engine;
    let store: serde_json::Value =
        serde_json::from_str(acme_json).context("This is not a Traefik acme.json: it is not JSON")?;
    let resolvers = store
        .as_object()
        .context("This is not a Traefik acme.json: expected one entry per certificate resolver")?;
    let decode = |field: &serde_json::Value, what: &str, name: &str| -> Result<String> {
        let text = field
            .as_str()
            .with_context(|| format!("The {} of {} is missing", what, name))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.trim())
            .with_context(|| format!("The {} of {} is not base64", what, name))?;
        String::from_utf8(bytes).with_context(|| format!("The {} of {} is not PEM", what, name))
    };

    let mut report = ImportReport::default();
    for resolver in resolvers.values() {
        let Some(certificates) = resolver.get("Certificates").and_then(|c| c.as_array()) else {
            continue;
        };
        for entry in certificates {
            let domain = &entry["domain"];
            let mut names: Vec<String> = domain["main"].as_str().map(String::from).into_iter().collect();
            if let Some(sans) = domain["sans"].as_array() {
                names.extend(sans.iter().filter_map(|s| s.as_str().map(String::from)));
            }
            let Some(main) = names.first().cloned() else {
                continue;
            };
            // The names this gateway can use are known before the entry is
            // decoded, so an entry with none of them is not even looked at.
            let mut wanted = Vec::new();
            for name in names {
                if name.starts_with("*.") {
                    report
                        .skipped
                        .push((name, "a wildcard, which HTTP-01 cannot renew".to_string()));
                } else if only.is_some_and(|domains| !domains.contains(&name)) {
                    report
                        .skipped
                        .push((name, "not one of the gateway's domains".to_string()));
                } else {
                    wanted.push(name);
                }
            }
            if wanted.is_empty() {
                continue;
            }
            // One entry Traefik left half-written (an order it never finished)
            // must not stop the import of the others, or the deploy.
            let decoded = decode(&entry["certificate"], "certificate", &main).and_then(|chain| {
                let key = decode(&entry["key"], "key", &main)?;
                let expires = crate::controller::acme::not_after(&chain)?;
                Ok((chain, key, expires))
            });
            let (chain, key, expires) = match decoded {
                Ok(d) => d,
                Err(e) => {
                    for name in wanted {
                        report.skipped.push((name, format!("{:#}", e)));
                    }
                    continue;
                }
            };
            let pem = format!(
                "{}
{}",
                chain.trim_end(),
                key.trim_end()
            ) + "
";
            for name in wanted {
                if expires <= now {
                    report.skipped.push((name, "expired".to_string()));
                    continue;
                }
                let secret = object_name(&crate::gateway_tls::secret_base(&name), pem.as_bytes());
                if docker.run(&["secret", "inspect", &secret])?.success {
                    report.already_there.push(name);
                    continue;
                }
                let output = docker.run_input(
                    &secret_create_args(&name, ISSUER_ACME, expires, &secret)
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    pem.as_bytes(),
                )?;
                if !output.success {
                    bail!("Cannot create the secret of {}: {}", name, output.stderr.trim());
                }
                report.imported.push(name);
            }
        }
    }
    Ok(report)
}

// --------------------------------------------------------- retire-legacy-stack

/// simpled 2.0.x ran the gateway in a stack called `ingress`. Its gateway task
/// holds ports 80 and 443 on the host, which the `simpled` stack's gateway
/// needs, and the stack owns the `simpled_control` network. So the first deploy
/// with the new names removes it, once, and waits until its containers and
/// networks are gone. Only a stack whose gateway carries simpled's label is
/// removed; any other `ingress` stack stops the deploy with what to do.
///
/// `images` are the new stack's: they are pulled first, while the old gateway
/// still serves, so an image that cannot be pulled (a tag that does not exist, a
/// registry that cannot be reached) stops the deploy before anything is removed.
pub fn retire_legacy_stack(
    docker: &dyn Docker,
    images: &[String],
    wait: std::time::Duration,
    poll: std::time::Duration,
) -> Result<bool> {
    let output = docker.run(&[
        "service",
        "inspect",
        LEGACY_GATEWAY_SERVICE,
        "--format",
        "{{index .Spec.Labels \"simpled.version\"}}",
    ])?;
    if !output.success {
        return Ok(false);
    }
    let version = output.stdout.trim();
    if version.is_empty() || version == "<no value>" {
        bail!(
            "A stack '{}' with a service {} holds the gateway's ports, but it carries no simpled.version label, so \
             simpled will not remove it. If an older simpled deployed it, remove it with `docker stack rm {}` and \
             run deploy.sh again; the gateway is down in between.",
            LEGACY_STACK,
            LEGACY_GATEWAY_SERVICE,
            LEGACY_STACK
        );
    }
    for image in images {
        eprintln!("Pulling {} before the '{}' stack is removed", image, LEGACY_STACK);
        let output = docker.run(&["pull", "--quiet", image])?;
        if !output.success {
            bail!(
                "Cannot pull {}, which the '{}' stack's gateway needs: {}. The '{}' stack is left as it is; \
                 fix the image (or set the variable it comes from) and run deploy.sh again.",
                image,
                STACK,
                output.stderr.trim(),
                LEGACY_STACK
            );
        }
    }
    eprintln!(
        "The gateway now runs in the '{}' stack. Removing the '{}' stack simpled {} deployed; \
         the gateway is down until the new one starts.",
        STACK, LEGACY_STACK, version
    );
    checked(docker, &["stack", "rm", LEGACY_STACK])?;
    let namespace = format!("label=com.docker.stack.namespace={}", LEGACY_STACK);
    let started = std::time::Instant::now();
    loop {
        let containers = checked(docker, &["ps", "-q", "--filter", &namespace])?;
        let networks = checked(docker, &["network", "ls", "-q", "--filter", &namespace])?;
        if containers.trim().is_empty() && networks.trim().is_empty() {
            return Ok(true);
        }
        if started.elapsed() >= wait {
            bail!(
                "The '{}' stack was removed, but its containers or networks were still there after {}; \
                 run deploy.sh again once `docker ps --filter {}` shows nothing",
                LEGACY_STACK,
                human(wait),
                namespace
            );
        }
        std::thread::sleep(poll);
    }
}

/// The images a stack file's services run, with `${VAR:-default}` resolved the
/// way `docker stack deploy` does it, from `lookup`. An image written any other
/// way is skipped: it is checked by the deploy itself.
pub fn stack_images(compose: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let Ok(stack) = serde_yaml::from_str::<serde_yaml::Value>(compose) else {
        return vec![];
    };
    let Some(services) = stack.get("services").and_then(|s| s.as_mapping()) else {
        return vec![];
    };
    let mut images = Vec::new();
    for service in services.values() {
        let Some(image) = service.get("image").and_then(|i| i.as_str()) else {
            continue;
        };
        let resolved = match image.strip_prefix("${").and_then(|rest| rest.strip_suffix('}')) {
            Some(inner) => match inner.split_once(":-") {
                Some((var, default)) => lookup(var).filter(|v| !v.is_empty()).unwrap_or(default.to_string()),
                None => continue,
            },
            None if image.contains('$') => continue,
            None => image.to_string(),
        };
        if !images.contains(&resolved) {
            images.push(resolved);
        }
    }
    images
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

// ---------------------------------------------------------------- lock

/// The Swarm config that is the cluster-wide deploy lock. Creating a config whose
/// name exists fails, and Raft keeps that consistent across every manager.
pub const LOCK_NAME: &str = "simpled_deploy_lock";

/// Who holds the lock, from its labels.
struct LockHolder {
    deployment: String,
    host: String,
    since: u64,
    token: String,
}

fn lock_holder(docker: &dyn Docker) -> Result<Option<LockHolder>> {
    let output = docker.run(&["config", "inspect", LOCK_NAME, "--format", "{{json .Spec.Labels}}"])?;
    if !output.success {
        return Ok(None);
    }
    let labels: serde_json::Value = serde_json::from_str(output.stdout.trim()).unwrap_or_default();
    let label = |key: &str| labels[key].as_str().unwrap_or("unknown").to_string();
    Ok(Some(LockHolder {
        deployment: label("simpled.lock.deployment"),
        host: label("simpled.lock.host"),
        since: labels["simpled.lock.since"]
            .as_str()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        token: label("simpled.lock.token"),
    }))
}

/// Timing of `lock`, apart so tests need not wait.
pub struct LockTiming {
    /// How long to wait for another deploy to release the lock.
    pub wait: Duration,
    pub poll: Duration,
    /// A lock older than this is reported as probably left by a deploy that died.
    pub stale_after: Duration,
}

/// Takes the swarm's deploy lock for `deployment`, waiting while another deploy
/// holds it. Returns the token `unlock` needs, so a deploy only ever releases its
/// own lock. `lock_file` is where the config's content is written first.
pub fn lock(
    docker: &dyn Docker,
    deployment: &str,
    host: &str,
    now: &dyn Fn() -> u64,
    lock_file: &Path,
    timing: &LockTiming,
) -> Result<String> {
    let started = Instant::now();
    let since = now();
    let token = {
        let seed = format!(
            "{}:{}:{}:{:?}",
            deployment,
            host,
            std::process::id(),
            std::time::SystemTime::now()
        );
        object_name("t", seed.as_bytes()).trim_start_matches("t_").to_string()
    };
    std::fs::write(
        lock_file,
        format!("Deploy of {} from {} since {}\n", deployment, host, since),
    )
    .with_context(|| format!("Cannot write {}", lock_file.display()))?;
    let file = lock_file.to_string_lossy();
    let labels = [
        format!("simpled.lock.deployment={}", deployment),
        format!("simpled.lock.host={}", host),
        format!("simpled.lock.since={}", since),
        format!("simpled.lock.token={}", token),
    ];

    let mut announced = false;
    loop {
        let mut args = vec!["config", "create"];
        for label in &labels {
            args.extend(["--label", label.as_str()]);
        }
        args.extend([LOCK_NAME, &file]);
        let created = docker.run(&args)?;
        if created.success {
            let _ = std::fs::remove_file(lock_file);
            eprintln!("Took the deploy lock.");
            return Ok(token);
        }
        if !created.stderr.contains("already exists") && !created.stderr.contains("AlreadyExists") {
            let _ = std::fs::remove_file(lock_file);
            bail!("Cannot take the deploy lock: {}", created.stderr.trim());
        }

        let Some(holder) = lock_holder(docker)? else {
            // Released between the two calls; try again right away.
            continue;
        };
        let age = Duration::from_secs(now().saturating_sub(holder.since));
        if started.elapsed() >= timing.wait {
            let _ = std::fs::remove_file(lock_file);
            let mut message = format!(
                "Another deploy holds the swarm's deploy lock: deployment '{}' from {}, for {}. \
                 Gave up after waiting {}; SIMPLED_LOCK_WAIT (seconds) waits longer.",
                holder.deployment,
                holder.host,
                human(age),
                human(timing.wait)
            );
            if age >= timing.stale_after {
                message.push_str(&format!(
                    "\nA deploy does not take that long, so that one probably died. If nothing is deploying, \
                     clear the lock with: ./simpled step unlock --force (or docker config rm {})",
                    LOCK_NAME
                ));
            }
            bail!(message);
        }
        if !announced {
            eprintln!(
                "Waiting for the deploy lock, held by deployment '{}' from {} for {}...",
                holder.deployment,
                holder.host,
                human(age)
            );
            announced = true;
        }
        std::thread::sleep(timing.poll);
    }
}

/// `45 seconds`, `1 minute`, `12 minutes`.
fn human(duration: Duration) -> String {
    let secs = duration.as_secs();
    match secs {
        1 => "1 second".to_string(),
        s if s < 60 => format!("{} seconds", s),
        60..=119 => "1 minute".to_string(),
        s => format!("{} minutes", s / 60),
    }
}

/// Releases the deploy lock if `token` is the one it was taken with, or whoever
/// holds it with `force`. A lock someone else holds is left alone.
pub fn unlock(docker: &dyn Docker, token: Option<&str>, force: bool) -> Result<()> {
    let Some(holder) = lock_holder(docker)? else {
        return Ok(());
    };
    if !force && token != Some(holder.token.as_str()) {
        eprintln!(
            "The deploy lock is held by deployment '{}' from {}, not by this deploy; left alone.",
            holder.deployment, holder.host
        );
        return Ok(());
    }
    checked(docker, &["config", "rm", LOCK_NAME])?;
    eprintln!("Released the deploy lock.");
    Ok(())
}

// ---------------------------------------------------------------- check-gateway

/// The gateway service of the ingress stack.
/// The stack the gateway and the controller run in, and their services.
pub const STACK: &str = "simpled";
pub const GATEWAY_SERVICE: &str = "simpled_gateway";
/// Before simpled 2.1 the stack was `ingress`, which is also the name of
/// Swarm's own routing-mesh network.
pub const LEGACY_STACK: &str = "ingress";
pub const LEGACY_GATEWAY_SERVICE: &str = "ingress_traefik";

/// Every deployment of an env spec redeploys the shared gateway from its own copy
/// of the env spec. Before it does, this compares what runs with what is about to
/// replace it:
///
/// - a gateway from a newer simpled is not replaced by an older one, unless
///   `allow_downgrade`, since the controller's image is pinned to that version;
/// - when the gateway's routing changes, the change is printed, so a deploy from
///   a stale checkout of the env spec shows in the log.
///
/// `lookup` gives the names `step sync-objects` exported for the plan's objects.
pub fn check_gateway(
    docker: &dyn Docker,
    plan: &Plan,
    deploy_dir: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
    allow_downgrade: bool,
) -> Result<()> {
    // The gateway a 2.0.x deploy left in the `ingress` stack counts too: it is
    // the one this deploy replaces.
    let mut output = docker.run(&["service", "inspect", GATEWAY_SERVICE, "--format", "{{json .Spec}}"])?;
    if !output.success {
        output = docker.run(&[
            "service",
            "inspect",
            LEGACY_GATEWAY_SERVICE,
            "--format",
            "{{json .Spec}}",
        ])?;
    }
    if !output.success {
        return Ok(()); // The first deploy: no gateway yet.
    }
    let spec: serde_json::Value =
        serde_json::from_str(output.stdout.trim()).context("Cannot read the gateway service's spec")?;
    let running_version = spec["Labels"]["simpled.version"].as_str().unwrap_or("");
    let deployed_by = spec["Labels"]["simpled.deployment"]
        .as_str()
        .unwrap_or("an unknown deployment");

    if let (Ok(running), Ok(ours)) = (
        semver::Version::parse(running_version),
        semver::Version::parse(&plan.simpled_version),
    ) {
        if running > ours {
            let message = format!(
                "The gateway was deployed by simpled {} (deployment '{}'), newer than this deployment's simpled {}. \
                 Deploying would put the older gateway back. Prepare this deployment with simpled {} or later, \
                 or run deploy.sh with SIMPLED_ALLOW_DOWNGRADE=1 to do it anyway.",
                running, deployed_by, ours, running
            );
            if !allow_downgrade {
                bail!(message);
            }
            eprintln!("Warning: {} SIMPLED_ALLOW_DOWNGRADE is set, continuing.", message);
        }
    }

    let running_configs = spec["TaskTemplate"]["ContainerSpec"]["Configs"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for object in plan.objects.iter().filter(|o| o.kind == ObjectKind::Config) {
        let Some(new_name) = lookup(&object.var) else {
            continue;
        };
        let prefix = format!("{}_", object.base);
        let Some(old_name) = running_configs
            .iter()
            .filter_map(|c| c["ConfigName"].as_str())
            .find(|name| name.starts_with(&prefix))
        else {
            continue;
        };
        if old_name == new_name {
            continue;
        }
        let data = checked(
            docker,
            &["config", "inspect", old_name, "--format", "{{json .Spec.Data}}"],
        )?;
        let old = serde_json::from_str::<String>(data.trim())
            .ok()
            .and_then(|b64| {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.decode(b64).ok()
            })
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let new = std::fs::read_to_string(deploy_dir.join(&object.file)).unwrap_or_default();
        eprintln!(
            "The gateway's {} changes (it was last deployed by deployment '{}'):",
            object.base.trim_start_matches("gateway_"),
            deployed_by
        );
        for line in line_changes(&old, &new, 40) {
            eprintln!("  {}", line);
        }
        eprintln!(
            "Every deployment of the env spec redeploys the gateway. If this checkout of the env spec is older \
             than the one '{}' was deployed from, these changes undo newer ones.",
            deployed_by
        );
    }
    Ok(())
}

/// Lines only in `old` (`- `) and only in `new` (`+ `), in their order, at most
/// `limit` of them. Not a minimal diff, but enough to see which routes changed.
fn line_changes(old: &str, new: &str, limit: usize) -> Vec<String> {
    let count = |text: &str| {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for line in text.lines() {
            *counts.entry(line.to_string()).or_default() += 1;
        }
        counts
    };
    let (mut in_new, mut in_old) = (count(new), count(old));
    let mut changes = Vec::new();
    for line in old.lines() {
        match in_new.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => changes.push(format!("- {}", line)),
        }
    }
    for line in new.lines() {
        match in_old.get_mut(line) {
            Some(n) if *n > 0 => *n -= 1,
            _ => changes.push(format!("+ {}", line)),
        }
    }
    if changes.len() > limit {
        let more = changes.len() - limit;
        changes.truncate(limit);
        changes.push(format!("... and {} more changed lines", more));
    }
    changes
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
        answers: Vec<(String, bool, String, String)>,
        calls: RefCell<Vec<String>>,
    }

    impl FakeDocker {
        fn new(answers: &[(&str, bool, &str)]) -> Self {
            let answers: Vec<(&str, bool, &str, &str)> =
                answers.iter().map(|(p, ok, out)| (*p, *ok, *out, "boom")).collect();
            Self::with_stderr(&answers)
        }

        /// Answers that also say what a failing command prints on stderr.
        fn with_stderr(answers: &[(&str, bool, &str, &str)]) -> Self {
            FakeDocker {
                answers: answers
                    .iter()
                    .map(|(p, ok, out, err)| (p.to_string(), *ok, out.to_string(), err.to_string()))
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
            let (success, stdout, stderr) = self
                .answers
                .iter()
                .find(|(prefix, _, _, _)| call.starts_with(prefix.as_str()))
                .map(|(_, ok, out, err)| (*ok, out.clone(), if *ok { String::new() } else { err.clone() }))
                .unwrap_or((true, String::new(), String::new()));
            Ok(DockerOutput {
                success,
                stdout,
                stderr,
            })
        }
    }

    fn plan(objects: Vec<PlanObject>) -> Plan {
        Plan {
            simpled_version: "2.0.0".to_string(),
            owner_label: "simpled.deployment=prod".to_string(),
            objects,
            jobs: vec![],
            tls: vec![],
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

    fn timing(wait_secs: u64) -> LockTiming {
        LockTiming {
            wait: Duration::from_secs(wait_secs),
            poll: Duration::ZERO,
            stale_after: Duration::from_secs(30 * 60),
        }
    }

    #[test]
    fn the_lock_is_taken_with_who_holds_it() {
        let dir = tempfile::tempdir().unwrap();
        let docker = FakeDocker::new(&[]);
        let token = lock(&docker, "shop", "ci-7", &|| 1000, &dir.path().join("lock"), &timing(60)).unwrap();
        let create = docker
            .calls()
            .into_iter()
            .find(|c| c.starts_with("config create"))
            .unwrap();
        assert!(
            create.contains(
                "--label simpled.lock.deployment=shop --label simpled.lock.host=ci-7 --label simpled.lock.since=1000"
            ),
            "{}",
            create
        );
        assert!(
            create.contains(&format!("--label simpled.lock.token={} simpled_deploy_lock", token)),
            "{}",
            create
        );
        assert!(!dir.path().join("lock").exists(), "the content file is cleaned up");
    }

    #[test]
    fn a_held_lock_fails_after_the_wait_naming_its_holder() {
        let dir = tempfile::tempdir().unwrap();
        let holder = r#"{"simpled.lock.deployment":"blog","simpled.lock.host":"ci-3","simpled.lock.since":"100","simpled.lock.token":"x"}"#;
        let docker = FakeDocker::with_stderr(&[
            ("config create", false, "", "Error response from controller: rpc error: code = AlreadyExists desc = config simpled_deploy_lock already exists"),
            ("config inspect simpled_deploy_lock", true, holder, ""),
        ]);
        // Held for 10 minutes: not stale yet.
        let err = lock(&docker, "shop", "ci-7", &|| 700, &dir.path().join("lock"), &timing(0))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("deployment 'blog' from ci-3, for 10 minutes. Gave up after waiting 0 seconds"),
            "{}",
            err
        );
        assert!(!err.contains("probably died"), "{}", err);

        // Held for an hour: probably left by a deploy that died.
        let err = lock(&docker, "shop", "ci-7", &|| 3700, &dir.path().join("lock"), &timing(0))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("probably died") && err.contains("./simpled step unlock --force"),
            "{}",
            err
        );
    }

    #[test]
    fn only_the_holder_releases_the_lock_unless_forced() {
        let holder = r#"{"simpled.lock.deployment":"blog","simpled.lock.host":"ci-3","simpled.lock.since":"100","simpled.lock.token":"abc"}"#;
        let docker = FakeDocker::new(&[("config inspect simpled_deploy_lock", true, holder)]);
        unlock(&docker, Some("other"), false).unwrap();
        assert!(!docker.calls().iter().any(|c| c.starts_with("config rm")));
        unlock(&docker, Some("abc"), false).unwrap();
        assert!(docker.calls().iter().any(|c| c == "config rm simpled_deploy_lock"));

        let forced = FakeDocker::new(&[("config inspect simpled_deploy_lock", true, holder)]);
        unlock(&forced, None, true).unwrap();
        assert!(forced.calls().iter().any(|c| c == "config rm simpled_deploy_lock"));

        // No lock at all: nothing to do.
        let none = FakeDocker::new(&[("config inspect", false, "")]);
        unlock(&none, Some("abc"), false).unwrap();
        assert_eq!(none.calls().len(), 1);
    }

    fn gateway_plan(version: &str) -> Plan {
        Plan {
            simpled_version: version.to_string(),
            owner_label: "simpled.scope=environment".to_string(),
            objects: vec![object(
                ObjectKind::Config,
                "gateway_dynamic_conf.yml",
                "ingress/traefik/dynamic_conf.yml",
                "G_2",
            )],
            jobs: vec![],
            tls: vec![],
        }
    }

    fn gateway_spec(version: &str, config: &str) -> String {
        format!(
            r#"{{"Labels":{{"simpled.version":"{}","simpled.deployment":"blog"}},"TaskTemplate":{{"ContainerSpec":{{"Configs":[{{"ConfigName":"{}"}}]}}}}}}"#,
            version, config
        )
    }

    #[test]
    fn an_older_simpled_does_not_replace_a_newer_gateway() {
        let dir = tempfile::tempdir().unwrap();
        let docker = FakeDocker::new(&[("service inspect simpled_gateway", true, &gateway_spec("2.1.0", "x"))]);
        let err = check_gateway(&docker, &gateway_plan("2.0.0"), dir.path(), &|_: &str| None, false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("deployed by simpled 2.1.0 (deployment 'blog'), newer than this deployment's simpled 2.0.0"),
            "{}",
            err
        );
        assert!(check_gateway(&docker, &gateway_plan("2.0.0"), dir.path(), &|_: &str| None, true).is_ok());
        assert!(check_gateway(&docker, &gateway_plan("2.1.0"), dir.path(), &|_: &str| None, false).is_ok());
    }

    #[test]
    fn the_first_deploy_has_no_gateway_to_check() {
        let docker = FakeDocker::new(&[("service inspect", false, "")]);
        let dir = tempfile::tempdir().unwrap();
        check_gateway(&docker, &gateway_plan("2.0.0"), dir.path(), &|_: &str| None, false).unwrap();
    }

    #[test]
    fn a_routing_change_is_shown_against_what_runs() {
        use base64::Engine;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ingress/traefik")).unwrap();
        std::fs::write(
            dir.path().join("ingress/traefik/dynamic_conf.yml"),
            "http:\n  routers:\n    shop: a\n",
        )
        .unwrap();
        let old = base64::engine::general_purpose::STANDARD.encode("http:\n  routers:\n    shop: a\n    blog: b\n");
        let docker = FakeDocker::new(&[
            (
                "service inspect",
                true,
                &gateway_spec("2.0.0", "gateway_dynamic_conf.yml_old"),
            ),
            (
                "config inspect gateway_dynamic_conf.yml_old",
                true,
                &format!("\"{}\"", old),
            ),
        ]);
        let lookup = |v: &str| (v == "G_2").then(|| "gateway_dynamic_conf.yml_new".to_string());
        check_gateway(&docker, &gateway_plan("2.0.0"), dir.path(), &lookup, false).unwrap();
        assert!(docker
            .calls()
            .iter()
            .any(|c| c.starts_with("config inspect gateway_dynamic_conf.yml_old")));
        assert_eq!(
            line_changes(
                "http:\n  routers:\n    shop: a\n    blog: b\n",
                "http:\n  routers:\n    shop: a\n",
                40
            ),
            vec!["-     blog: b"]
        );
    }

    fn tls_plan() -> Plan {
        Plan {
            simpled_version: "2.0.0".to_string(),
            owner_label: "simpled.scope=environment".to_string(),
            objects: vec![],
            jobs: vec![],
            tls: vec![crate::deploy_plan::PlanTls {
                domain: "shop.example.com".to_string(),
                var: "SIMPLED_TLS_1".to_string(),
            }],
        }
    }

    #[test]
    fn an_issued_certificate_wins_over_a_newer_placeholder() {
        let docker = FakeDocker::new(&[
            (
                "secret ls -q --filter label=simpled.tls.domain=shop.example.com",
                true,
                "a
b
c
",
            ),
            (
                "secret inspect",
                true,
                "a|tls_shop.example.com_111|acme|1800000000|11
                 b|tls_shop.example.com_222|acme|1790000000|12
                 c|tls_shop.example.com_333|self-signed|0|13
",
            ),
        ]);
        assert_eq!(
            tls_secrets(&docker, &tls_plan()).unwrap(),
            vec![("SIMPLED_TLS_1".to_string(), "tls_shop.example.com_111".to_string())]
        );
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("secret create")));
    }

    #[test]
    fn a_domain_without_a_certificate_gets_a_labelled_placeholder() {
        let docker = FakeDocker::new(&[("secret ls", true, "")]);
        let exports = tls_secrets(&docker, &tls_plan()).unwrap();
        let name = &exports[0].1;
        assert!(name.starts_with("tls_shop.example.com_"), "{}", name);
        let calls = docker.calls.borrow();
        let create = calls.iter().find(|c| c.starts_with("secret create")).unwrap();
        assert_eq!(
            create,
            &format!(
                "secret create --label simpled.scope=environment --label simpled.tls.domain=shop.example.com --label simpled.tls.issuer=self-signed --label simpled.tls.not_after=0 {} -",
                name
            )
        );
    }

    /// A Traefik acme.json entry: base64 of the PEM chain and of the key.
    fn acme_entry(main: &str, sans: &[&str], year: i32) -> serde_json::Value {
        use base64::Engine;
        let mut names = vec![main.to_string()];
        names.extend(sans.iter().map(|s| s.to_string()));
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(names).unwrap();
        params.not_before = rcgen::date_time_ymd(year - 1, 1, 1);
        params.not_after = rcgen::date_time_ymd(year, 1, 1);
        let cert = params.self_signed(&key).unwrap();
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        serde_json::json!({
            "domain": {"main": main, "sans": sans},
            "certificate": b64(&cert.pem()),
            "key": b64(&key.serialize_pem()),
            "Store": "default",
        })
    }

    const NOW_2026: u64 = 1_793_000_000;

    #[test]
    fn every_name_of_an_imported_certificate_gets_its_own_secret() {
        let store = serde_json::json!({
            "myresolver": {
                "Account": {"Email": "ops@example.com"},
                "Certificates": [
                    acme_entry("shop.example.com", &["www.shop.example.com"], 2027),
                    acme_entry("old.example.com", &[], 2025),
                    acme_entry("*.example.com", &[], 2027),
                ],
            }
        })
        .to_string();
        let docker = FakeDocker::new(&[("secret inspect", false, "")]);
        let report = import_acme_json(&docker, &store, None, NOW_2026).unwrap();
        assert_eq!(report.imported, vec!["shop.example.com", "www.shop.example.com"]);
        let skipped: Vec<(&str, &str)> = report.skipped.iter().map(|(n, w)| (n.as_str(), w.as_str())).collect();
        assert_eq!(
            skipped,
            vec![
                ("old.example.com", "expired"),
                ("*.example.com", "a wildcard, which HTTP-01 cannot renew")
            ]
        );
        let calls = docker.calls.borrow();
        let creates: Vec<&String> = calls.iter().filter(|c| c.starts_with("secret create")).collect();
        assert_eq!(creates.len(), 2);
        assert!(
            creates[0].contains("--label simpled.tls.domain=shop.example.com"),
            "{}",
            creates[0]
        );
        assert!(creates[0].contains("--label simpled.tls.issuer=acme"), "{}", creates[0]);
        assert!(
            creates[0].contains("--label simpled.tls.not_after=1798761600"),
            "{}",
            creates[0]
        );
    }

    #[test]
    fn an_import_is_limited_to_the_gateways_domains_and_creates_nothing_twice() {
        let store = serde_json::json!({
            "myresolver": {"Certificates": [
                acme_entry("shop.example.com", &[], 2027),
                acme_entry("gone.example.com", &[], 2027),
            ]}
        })
        .to_string();
        let docker = FakeDocker::new(&[("secret inspect", true, "")]);
        let only = vec!["shop.example.com".to_string()];
        let report = import_acme_json(&docker, &store, Some(&only), NOW_2026).unwrap();
        assert_eq!(report.already_there, vec!["shop.example.com"]);
        assert_eq!(report.skipped[0].0, "gone.example.com");
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("secret create")));
    }

    #[test]
    fn a_file_that_is_not_an_acme_json_says_so() {
        let docker = FakeDocker::new(&[]);
        let err = import_acme_json(&docker, "not json", None, NOW_2026).unwrap_err();
        assert!(err.to_string().contains("not a Traefik acme.json"), "{}", err);
    }

    #[test]
    fn the_downgrade_guard_also_reads_a_gateway_in_the_old_ingress_stack() {
        let docker = FakeDocker::new(&[
            ("service inspect simpled_gateway", false, ""),
            ("service inspect ingress_traefik", true, &gateway_spec("9.0.0", "x")),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let err = check_gateway(&docker, &gateway_plan("2.1.0"), dir.path(), &|_| None, false).unwrap_err();
        assert!(err.to_string().contains("simpled 9.0.0"), "{}", err);
    }

    #[test]
    fn only_a_simpled_made_ingress_stack_is_retired() {
        let quick = std::time::Duration::from_millis(1);
        // None there: nothing to do.
        let docker = FakeDocker::new(&[("service inspect ingress_traefik", false, "")]);
        assert!(!retire_legacy_stack(&docker, &[], quick, quick).unwrap());
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("stack rm")));

        // simpled's: removed, then waited for.
        let docker = FakeDocker::new(&[("service inspect ingress_traefik", true, "2.0.1\n")]);
        assert!(retire_legacy_stack(&docker, &[], quick, quick).unwrap());
        let calls = docker.calls.borrow();
        assert!(calls.contains(&"stack rm ingress".to_string()), "{:?}", calls);
        assert!(calls
            .iter()
            .any(|c| c == "ps -q --filter label=com.docker.stack.namespace=ingress"));
        drop(calls);

        // Someone else's: left alone, with what to do.
        let docker = FakeDocker::new(&[("service inspect ingress_traefik", true, "<no value>\n")]);
        let err = retire_legacy_stack(&docker, &[], quick, quick).unwrap_err();
        assert!(err.to_string().contains("carries no simpled.version label"), "{}", err);
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("stack rm")));
    }

    #[test]
    fn retiring_gives_up_when_the_old_stack_lingers() {
        let quick = std::time::Duration::from_millis(1);
        let docker = FakeDocker::new(&[
            ("service inspect ingress_traefik", true, "2.0.1"),
            ("ps -q", true, "abc123\n"),
        ]);
        let err = retire_legacy_stack(&docker, &[], quick, quick).unwrap_err();
        assert!(err.to_string().contains("still there"), "{}", err);
    }

    #[test]
    fn the_new_images_are_pulled_while_the_old_stack_still_serves() {
        let quick = std::time::Duration::from_millis(1);
        let images = vec!["traefik:v2.10".to_string(), "ghcr.io/vkubiv/simpled:2.1.0".to_string()];
        let docker = FakeDocker::new(&[("service inspect ingress_traefik", true, "2.0.1\n")]);
        assert!(retire_legacy_stack(&docker, &images, quick, quick).unwrap());
        let calls = docker.calls.borrow();
        let position = |prefix: &str| calls.iter().position(|c| c.starts_with(prefix)).unwrap();
        assert!(
            position("pull --quiet traefik:v2.10") < position("stack rm ingress"),
            "{:?}",
            calls
        );
        assert!(position("pull --quiet ghcr.io/vkubiv/simpled:2.1.0") < position("stack rm ingress"));
        drop(calls);

        // An image that cannot be pulled: nothing is removed.
        let docker = FakeDocker::new(&[
            ("service inspect ingress_traefik", true, "2.0.1\n"),
            ("pull --quiet ghcr.io", false, ""),
        ]);
        let err = retire_legacy_stack(&docker, &images, quick, quick).unwrap_err();
        assert!(
            err.to_string().contains("Cannot pull ghcr.io/vkubiv/simpled:2.1.0"),
            "{}",
            err
        );
        assert!(err.to_string().contains("boom"), "{}", err);
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("stack rm")));

        // No legacy stack: nothing is pulled either.
        let docker = FakeDocker::new(&[("service inspect ingress_traefik", false, "")]);
        assert!(!retire_legacy_stack(&docker, &images, quick, quick).unwrap());
        assert!(!docker.calls.borrow().iter().any(|c| c.starts_with("pull")));
    }

    #[test]
    fn stack_images_resolve_variables_the_way_docker_stack_deploy_does() {
        let compose = concat!(
            "services:\n",
            "  traefik:\n    image: traefik:v2.10\n",
            "  controller:\n    image: \"${SIMPLED_CONTROLLER_IMAGE:-ghcr.io/vkubiv/simpled:2.1.0}\"\n",
            "  other:\n    image: ${NO_DEFAULT}\n",
            "  again:\n    image: traefik:v2.10\n",
        );
        let none = |_: &str| None;
        assert_eq!(
            stack_images(compose, &none),
            vec!["traefik:v2.10", "ghcr.io/vkubiv/simpled:2.1.0"]
        );
        let set = |name: &str| (name == "SIMPLED_CONTROLLER_IMAGE").then(|| "registry.local/simpled:dev".to_string());
        assert_eq!(
            stack_images(compose, &set),
            vec!["traefik:v2.10", "registry.local/simpled:dev"]
        );
        assert!(stack_images("not: [yaml", &none).is_empty());
    }

    #[test]
    fn a_half_written_entry_is_skipped_and_the_others_are_imported() {
        let mut broken = acme_entry("pending.example.com", &[], 2027);
        broken["certificate"] = serde_json::Value::String(String::new());
        let mut foreign = acme_entry("gone.example.com", &[], 2027);
        foreign["certificate"] = serde_json::Value::String("not base64!".to_string());
        let store = serde_json::json!({
            "myresolver": {"Certificates": [broken, foreign, acme_entry("shop.example.com", &[], 2027)]}
        })
        .to_string();
        let docker = FakeDocker::new(&[("secret inspect", false, "")]);
        let only = vec!["shop.example.com".to_string(), "pending.example.com".to_string()];
        let report = import_acme_json(&docker, &store, Some(&only), NOW_2026).unwrap();
        assert_eq!(report.imported, vec!["shop.example.com"]);
        assert_eq!(report.skipped.len(), 2, "{:?}", report.skipped);
        assert_eq!(report.skipped[0].0, "pending.example.com");
        assert!(report.skipped[0].1.contains("not PEM"), "{}", report.skipped[0].1);
        // Not one of the gateway's domains: never decoded, so its garbage is not an error.
        assert_eq!(
            report.skipped[1],
            (
                "gone.example.com".to_string(),
                "not one of the gateway's domains".to_string()
            )
        );
    }
}
