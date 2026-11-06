use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

mod app_bundle;
mod bundle_repo;
mod client;
mod controller;
mod deploy_plan;
mod deploy_steps;
mod docker_compose;
mod docker_generator;
mod docker_resources;
mod docs;
mod env_loader;
mod gateway_tls;
mod k8s_generator;
mod local_ingress;
mod ports;
mod resolved_spec;
mod resolver;
mod run_local;
mod run_test;
mod secret_fetch;
mod spec;
mod spec_loader;
mod spec_yaml;
mod swarm_objects;
mod target_binary;
mod test_spec;
#[cfg(test)]
mod test_support;
mod transform;
mod updater;
mod validator;
mod version_endpoint;

#[derive(Parser)]
#[command(name = "simpled", version)]
#[command(about = "A CLI tool for simplified k8s manifest generation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// App bundle related commands
    AppBundle {
        #[command(subcommand)]
        command: AppBundleCommands,
    },
    /// Prepare deployment (e.g. generate k8s manifests)
    PrepareDeployment {
        deployment_name: String,

        #[command(flatten)]
        bundle: bundle_repo::BundleArgs,

        /// Swarm: the simpled binary to ship next to deploy.sh, built for the deploy
        /// target and of this same version. Needed only when this machine is not
        /// Linux on the target's CPU and the version has no GitHub release to
        /// download it from. Also read from SIMPLED_TARGET_BINARY.
        #[arg(long)]
        target_binary: Option<std::path::PathBuf>,
    },

    /// Steps of a Swarm deploy, run by the deploy.sh that prepare-deployment
    /// writes, with the plan.json files next to it
    #[command(hide = true)]
    Step {
        #[command(subcommand)]
        command: StepCommands,
    },

    /// Every deployment's version, health and services, from the simpled controller.
    /// Reads the API's address from the env spec here (controller.hosts)
    /// and the token from SIMPLED_TOKEN
    Status {
        /// The API's base instead, e.g. https://ops.example.com/api/v1
        #[arg(long)]
        url: Option<String>,
        /// Print the API's JSON answer
        #[arg(long)]
        json: bool,
    },

    /// A service's log, from the simpled controller. On Swarm, every task's lines
    Logs {
        deployment: String,
        service: String,
        /// Keep printing new lines
        #[arg(short, long)]
        follow: bool,
        /// Lines from the end of each task's log, or `all` (default 200)
        #[arg(long)]
        tail: Option<String>,
        /// Only lines newer than this: 30s, 10m, 2h, 1d, or Unix seconds
        #[arg(long)]
        since: Option<String>,
        /// Prefix every line with its time
        #[arg(long)]
        timestamps: bool,
        /// The API's base instead of the one in the env spec
        #[arg(long)]
        url: Option<String>,
    },

    /// Waits until a deployment reports a version as healthy and converged, on
    /// its primary host's public /.well-known/simpled/health. For CI, after
    /// deploy.sh. Needs no token
    Wait {
        deployment: String,
        /// The version just deployed
        #[arg(long)]
        version: String,
        /// Give up after this long
        #[arg(long, default_value = "10m")]
        timeout: String,
        /// Between two checks
        #[arg(long, default_value = "5s")]
        interval: String,
        /// The health URL instead of the one from the env spec
        #[arg(long)]
        url: Option<String>,
        /// Accept any certificate: for a staging or private ACME server
        #[arg(long)]
        insecure: bool,
    },

    /// The gateway's certificates on Docker Swarm
    Tls {
        #[command(subcommand)]
        command: TlsCommands,
    },

    /// The environment's controller, next to the gateway: answers the version and
    /// health of every deployment. Run by the generated stack, not by hand
    #[command(hide = true)]
    Controller {
        /// The controller.json that prepare-deployment writes
        #[arg(long)]
        config: std::path::PathBuf,
        #[arg(long, default_value = "0.0.0.0:8080")]
        listen: String,
    },

    /// Used for local development and tests
    Local {
        #[command(subcommand)]
        command: LocalCommands,
    },

    /// Bring a local deployment up, run a test suite from testspec.yaml against
    /// it, tear it down, and exit with the suite's status
    Test {
        /// Suite to run. Every suite, in name order, when omitted.
        suite: Option<String>,

        #[arg(long)]
        path: Option<String>,

        /// Leave the stack running after the suite, to debug a failure
        #[arg(long)]
        keep: bool,

        /// Run the suite against whatever is already up: no compose, no
        /// gateway, no readiness wait
        #[arg(long)]
        no_up: bool,

        /// Print the services' logs after every suite, not only a failing one
        #[arg(long)]
        logs: bool,

        /// Instance to run the stack as: a number, or `auto` for the first one
        /// whose named ports are all free. Defaults to SIMPLED_INSTANCE, then
        /// .simpled-instance, then 0.
        #[arg(long)]
        instance: Option<String>,

        /// Run the app services from a released bundle: the images CI published
        /// for its version, through the env spec's `registry:`, instead of the
        /// local `:latest` ones
        #[command(flatten)]
        bundle: bundle_repo::BundleArgs,
    },

    /// Print the documentation embedded in this binary
    ///
    /// `simpled docs` lists the topics, `simpled docs <topic>` prints one, and
    /// `simpled docs search <query>` searches all of them.
    #[command(long_about = concat!(
        "Print the documentation embedded in this binary.\n\n",
        "  simpled docs                        list the available topics\n",
        "  simpled docs <topic>                print a topic\n",
        "  simpled docs <topic> --outline      list a topic's section headings\n",
        "  simpled docs <topic> --section <s>  print one section of a topic\n",
        "  simpled docs search <query>         search every topic\n\n",
        "Topics: agent, tutorial, reference, examples, cicd. An unambiguous prefix works.",
    ))]
    Docs {
        /// A topic name, `list`, or `search` followed by a query
        #[arg(value_name = "TOPIC_OR_SEARCH")]
        args: Vec<String>,

        /// Print only the section whose heading matches this
        #[arg(long)]
        section: Option<String>,

        /// List the topic's section headings instead of its text
        #[arg(long)]
        outline: bool,
    },

    /// Write an agent skill file into a project so coding agents discover these docs
    InitAgent {
        /// Project directory to write into (default: current dir)
        #[arg(long)]
        path: Option<String>,

        /// Overwrite an existing skill file
        #[arg(long)]
        force: bool,

        /// Print the skill to stdout instead of writing it
        #[arg(long)]
        stdout: bool,
    },

    /// Check for a new release and update the binary
    Update {
        /// Only check for updates without installing
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum TlsCommands {
    /// Turns the certificates in a Traefik acme.json (simpled 2.0.x kept it in
    /// docker-deploy/letsencrypt/) into the gateway's certificate secrets, so
    /// the controller issues nothing new. Run it on a swarm manager
    Import {
        /// The acme.json
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum StepCommands {
    /// multi_node: false. Fails unless exactly one node accepts tasks, and prints
    /// `export SIMPLED_NODE_ID=...` for the constraint every service carries
    CheckNodes,
    /// Creates the plan's configs and secrets under their content-addressed names,
    /// and prints an `export` line naming each one
    SyncObjects { plan: std::path::PathBuf },
    /// Removes the plan's configs and secrets older than the previous deploy
    PruneObjects { plan: std::path::PathBuf },
    /// Runs one job of the plan to completion
    RunJob { plan: std::path::PathBuf, job: String },
    /// Takes the swarm's deploy lock for a deployment, waiting while another
    /// deploy holds it, and prints `export SIMPLED_LOCK_TOKEN=...`
    Lock { deployment: String },
    /// Releases the deploy lock this deploy took (SIMPLED_LOCK_TOKEN), or with
    /// --force whoever's it is
    Unlock {
        #[arg(long)]
        force: bool,
    },
    /// Before the gateway is redeployed: refuses to replace a newer simpled's
    /// gateway, and shows how its routing changes
    CheckGateway { plan: std::path::PathBuf },
    /// The certificate secret of each of the gateway's domains, creating a
    /// self-signed placeholder for a domain that has none yet
    TlsSecrets { plan: std::path::PathBuf },
    /// Removes the `ingress` stack a simpled 2.0.x deploy left, whose gateway
    /// holds the ports the `simpled` stack needs
    RetireLegacyStack,
    /// Imports the gateway's domains from a 2.0.x acme.json, then renames it to
    /// acme.json.imported so later deploys do not import it again
    TlsImport {
        plan: std::path::PathBuf,
        file: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
enum LocalCommands {
    Run {
        #[arg(short, long)]
        exclude: Option<Vec<String>>,

        #[arg(long)]
        path: Option<String>,

        /// Deployment to run. Required when the env spec defines more than one.
        #[arg(long)]
        deployment: Option<String>,

        /// Address the local gateway listens on. The default keeps a stack that
        /// carries real secrets off the network; pass 0.0.0.0 to reach it from
        /// another device.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Instance to run: a number, shifting every named port by
        /// `instance * port_step`. Defaults to SIMPLED_INSTANCE, then
        /// .simpled-instance, then 0.
        #[arg(long)]
        instance: Option<String>,

        /// Run the app services from a released bundle: the images CI published
        /// for its version, through the env spec's `registry:`, instead of the
        /// local `:latest` ones
        #[command(flatten)]
        bundle: bundle_repo::BundleArgs,
    },
    /// Run the gateway and only extra services (no app services)
    OnlyExtra {
        #[arg(long)]
        path: Option<String>,

        /// Deployment to run. Required when the env spec defines more than one.
        #[arg(long)]
        deployment: Option<String>,

        /// Address the local gateway listens on. The default keeps a stack that
        /// carries real secrets off the network; pass 0.0.0.0 to reach it from
        /// another device.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Instance to run: a number, shifting every named port by
        /// `instance * port_step`. Defaults to SIMPLED_INSTANCE, then
        /// .simpled-instance, then 0.
        #[arg(long)]
        instance: Option<String>,
    },
    /// Regenerate local_env configuration without running the gateway or docker compose
    GenerateConfig {
        #[arg(long)]
        path: Option<String>,

        /// Deployment to generate config for. Required when the env spec defines more than one.
        #[arg(long)]
        deployment: Option<String>,
        /// Instance to run: a number, shifting every named port by
        /// `instance * port_step`. Defaults to SIMPLED_INSTANCE, then
        /// .simpled-instance, then 0.
        #[arg(long)]
        instance: Option<String>,

        /// Generate for the app services of a released bundle, as `local run
        /// --app-bundle` runs them
        #[command(flatten)]
        bundle: bundle_repo::BundleArgs,
    },
}

#[derive(Subcommand)]
enum AppBundleCommands {
    Verify,
    Version,
    Create {
        #[arg(short, long)]
        registry: Option<String>,
        #[arg(long)]
        push_images: bool,
        /// Do not create missing Amazon ECR repositories before pushing. ECR, unlike
        /// most registries, rejects a push to a repository that does not exist yet.
        #[arg(long)]
        no_create_repos: bool,
        #[arg(long)]
        upload: Option<String>,
        #[arg(long)]
        upload_bundle_to: Option<String>,
        #[arg(long)]
        github_repo: Option<String>,
        #[arg(long)]
        github_tag_prefix: Option<String>,
        /// Label appended to the app version for side-branch builds: `big-refactor` turns
        /// 1.0.2 into 1.0.2+big-refactor, tagging images and the bundle 1.0.2-big-refactor.
        /// Characters outside [A-Za-z0-9] are replaced with '-', so a branch name works as is.
        #[arg(long)]
        version_suffix: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::AppBundle { command } => match command {
            AppBundleCommands::Verify => {
                verify_command()?;
            }
            AppBundleCommands::Version => {
                version_command()?;
            }
            AppBundleCommands::Create {
                registry,
                push_images,
                no_create_repos,
                upload,
                upload_bundle_to,
                github_repo,
                github_tag_prefix,
                version_suffix,
            } => {
                app_bundle::create_app_bundle(
                    registry,
                    *push_images,
                    !*no_create_repos,
                    upload,
                    upload_bundle_to,
                    github_repo,
                    github_tag_prefix,
                    version_suffix,
                )?;
            }
        },
        Commands::PrepareDeployment {
            deployment_name,
            bundle,
            target_binary,
        } => {
            prepare_deployment_command(deployment_name, bundle, target_binary.as_deref())?;
        }
        Commands::Step { command } => {
            step_command(command)?;
        }
        Commands::Status { url, json } => {
            let base = match url {
                Some(url) => url.trim_end_matches('/').to_string(),
                None => client::api_base(&spec_loader::load_env_spec(Path::new("."), None, 0)?)?,
            };
            client::status(&base, *json)?;
        }
        Commands::Logs {
            deployment,
            service,
            follow,
            tail,
            since,
            timestamps,
            url,
        } => {
            let base = match url {
                Some(url) => url.trim_end_matches('/').to_string(),
                None => client::api_base(&spec_loader::load_env_spec(Path::new("."), None, 0)?)?,
            };
            client::logs(
                &base,
                &client::LogArgs {
                    deployment,
                    service,
                    follow: *follow,
                    tail: tail.as_deref(),
                    since: since.as_deref(),
                    timestamps: *timestamps,
                },
            )?;
        }
        Commands::Wait {
            deployment,
            version,
            timeout,
            interval,
            url,
            insecure,
        } => {
            let url = match url {
                Some(url) => url.clone(),
                None => client::health_url(
                    &spec_loader::load_env_spec(Path::new("."), Some(deployment), 0)?,
                    deployment,
                )?,
            };
            client::wait(
                &url,
                version,
                client::parse_duration(timeout)?,
                client::parse_duration(interval)?,
                *insecure,
            )?;
        }
        Commands::Tls {
            command: TlsCommands::Import { file },
        } => {
            let text = std::fs::read_to_string(file).with_context(|| format!("Cannot read {}", file.display()))?;
            let report = deploy_steps::import_acme_json(&deploy_steps::DockerCli, &text, None, controller::unix_now())?;
            print_import_report(&report);
        }
        Commands::Controller { config, listen } => {
            controller::run(config, listen)?;
        }
        Commands::Local { command } => {
            local(command)?;
        }
        Commands::Test {
            suite,
            path,
            keep,
            no_up,
            logs,
            instance,
            bundle,
        } => {
            let root = path.as_ref().map(Path::new).unwrap_or(Path::new("."));
            let options = run_test::TestOptions {
                keep: *keep,
                no_up: *no_up,
                logs: *logs,
                instance: instance.clone(),
                bundle: bundle.clone(),
            };
            let code = run_test::run(root, suite.as_deref(), &options)?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Commands::Docs { args, section, outline } => {
            docs_command(args, section, *outline)?;
        }
        Commands::InitAgent { path, force, stdout } => {
            docs::init_agent(path.as_deref(), *force, *stdout)?;
        }
        Commands::Update { check } => {
            updater::check_and_update(*check)?;
        }
    }
    Ok(())
}

fn docs_command(args: &[String], section: &Option<String>, outline: bool) -> Result<()> {
    match args.split_first() {
        None => {
            docs::list();
            Ok(())
        }
        Some((first, rest)) if first == "list" && rest.is_empty() => {
            docs::list();
            Ok(())
        }
        Some((first, rest)) if first == "search" => docs::search(&rest.join(" ")),
        Some((topic, [])) => docs::show(topic, section.as_deref(), outline),
        Some((topic, rest)) => bail!(
            "Unexpected arguments after the topic '{}': {}. Did you mean `simpled docs search {}`?",
            topic,
            rest.join(" "),
            rest.join(" ")
        ),
    }
}

fn verify_command() -> Result<()> {
    let app_spec = spec_loader::load_app_spec(Path::new("."), None)?;
    println!(
        "Successfully validated appspec: {} v{}",
        app_spec.name, app_spec.version
    );
    Ok(())
}

fn version_command() -> Result<()> {
    let app_spec = spec_loader::load_app_spec(Path::new("."), None)?;
    println!("{}", app_spec.version);
    Ok(())
}

fn print_import_report(report: &deploy_steps::ImportReport) {
    for name in &report.imported {
        eprintln!("Imported the certificate of {}", name);
    }
    for name in &report.already_there {
        eprintln!("The certificate of {} was imported before", name);
    }
    for (name, why) in &report.skipped {
        eprintln!("Skipped {}: {}", name, why);
    }
    if report.imported.is_empty() && report.already_there.is_empty() {
        eprintln!("No certificate imported");
    }
}

fn step_command(command: &StepCommands) -> Result<()> {
    use deploy_plan::Plan;
    use deploy_steps::{export_line, DockerCli};

    // The plan and the binary come from the same prepare-deployment, so a
    // mismatch means files from two deployments were mixed on the target.
    let read_plan = |path: &std::path::Path| -> Result<Plan> {
        let plan = Plan::read(path)?;
        let running = env!("CARGO_PKG_VERSION");
        if plan.simpled_version != running {
            bail!(
                "{} was prepared by simpled {}, but this is simpled {}. Run the deploy.sh and simpled binary \
                 that came with the plan, from the same prepare-deployment",
                path.display(),
                plan.simpled_version,
                running
            );
        }
        Ok(plan)
    };
    let docker = DockerCli;
    let deploy_dir = std::env::current_dir().context("Cannot read the current directory")?;

    match command {
        StepCommands::CheckNodes => {
            println!(
                "{}",
                export_line("SIMPLED_NODE_ID", &deploy_steps::check_nodes(&docker)?)
            );
        }
        StepCommands::SyncObjects { plan } => {
            for (var, name) in deploy_steps::sync_objects(&docker, &read_plan(plan)?, &deploy_dir)? {
                println!("{}", export_line(&var, &name));
            }
        }
        StepCommands::PruneObjects { plan } => {
            deploy_steps::prune_objects(&docker, &read_plan(plan)?)?;
        }
        StepCommands::RunJob { plan, job } => {
            let plan = read_plan(plan)?;
            let job = plan.job(job)?;
            let timeout = match std::env::var("JOB_TIMEOUT") {
                Ok(value) => value
                    .parse::<u64>()
                    .with_context(|| format!("JOB_TIMEOUT '{}' is not a number of seconds", value))?,
                Err(_) => job.timeout_seconds.into(),
            };
            let dir = deploy_dir.to_string_lossy().into_owned();
            let lookup = move |name: &str| {
                std::env::var(name)
                    .ok()
                    .or_else(|| (name == "DEPLOY_DIR").then(|| dir.clone()))
            };
            deploy_steps::run_job(
                &docker,
                job,
                &lookup,
                std::time::Duration::from_secs(timeout),
                std::time::Duration::from_secs(2),
            )?;
        }
        StepCommands::Lock { deployment } => {
            let wait = match std::env::var("SIMPLED_LOCK_WAIT") {
                Ok(value) => value
                    .parse::<u64>()
                    .with_context(|| format!("SIMPLED_LOCK_WAIT '{}' is not a number of seconds", value))?,
                Err(_) => 600,
            };
            let host = ["HOSTNAME", "COMPUTERNAME"]
                .iter()
                .find_map(|v| std::env::var(v).ok().filter(|h| !h.is_empty()))
                .or_else(|| {
                    std::process::Command::new("hostname")
                        .output()
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                        .filter(|h| !h.is_empty())
                })
                .unwrap_or_else(|| "an unknown host".to_string());
            let now = || {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default()
            };
            let token = deploy_steps::lock(
                &docker,
                deployment,
                &host,
                &now,
                &deploy_dir.join(".simpled-lock"),
                &deploy_steps::LockTiming {
                    wait: std::time::Duration::from_secs(wait),
                    poll: std::time::Duration::from_secs(5),
                    stale_after: std::time::Duration::from_secs(30 * 60),
                },
            )?;
            println!("{}", export_line("SIMPLED_LOCK_TOKEN", &token));
        }
        StepCommands::Unlock { force } => {
            let token = std::env::var("SIMPLED_LOCK_TOKEN").ok();
            deploy_steps::unlock(&docker, token.as_deref(), *force)?;
        }
        StepCommands::TlsImport { plan: plan_path, file } => {
            let plan = read_plan(plan_path)?;
            let domains: Vec<String> = plan.tls.iter().map(|t| t.domain.clone()).collect();
            let done = file.with_extension("json.imported");
            // Traefik wrote the file as root, mode 0600, and often its directory
            // too; a deploy user that is only in the docker group can neither
            // read it nor rename it. Both are checked before anything is
            // imported, so the way out is one command, not a half-done import.
            let as_root = format!(
                "sudo ./simpled step tls-import {} {}",
                plan_path.display(),
                file.display()
            );
            let text = match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => bail!(
                    "Cannot read {}: {}. Traefik (simpled 2.0.x) wrote it as root. Import it once with `{}`, which \
                     also renames it to {}; or move the file away to start with new certificates. Then run deploy.sh \
                     again.",
                    file.display(),
                    e,
                    as_root,
                    done.display()
                ),
                Err(e) => return Err(e).with_context(|| format!("Cannot read {}", file.display())),
            };
            if let Some(dir) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
                let probe = dir.join(".simpled-write-check");
                match std::fs::write(&probe, b"") {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&probe);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => bail!(
                        "{} is readable, but {} cannot be written, so the file could not be renamed to {} after the \
                         import, and every deploy would import it again. Import it once with `{}`, then run deploy.sh \
                         again.",
                        file.display(),
                        dir.display(),
                        done.display(),
                        as_root
                    ),
                    Err(_) => {}
                }
            }
            let report = deploy_steps::import_acme_json(&docker, &text, Some(&domains), controller::unix_now())?;
            print_import_report(&report);
            std::fs::rename(file, &done)
                .with_context(|| format!("Cannot rename {} to {}", file.display(), done.display()))?;
            eprintln!("Renamed {} to {}", file.display(), done.display());
        }
        StepCommands::TlsSecrets { plan } => {
            let plan = read_plan(plan)?;
            for (var, name) in deploy_steps::tls_secrets(&docker, &plan)? {
                println!("{}", export_line(&var, &name));
            }
        }
        StepCommands::RetireLegacyStack => {
            // The new stack's images are pulled while the old gateway still
            // serves; a stack file that is not there yet means nothing to pull.
            let compose = deploy_dir.join(docker_generator::GATEWAY_STACK_FILE);
            let images = match std::fs::read_to_string(&compose) {
                Ok(text) => deploy_steps::stack_images(&text, &|name| std::env::var(name).ok()),
                Err(_) => vec![],
            };
            deploy_steps::retire_legacy_stack(
                &docker,
                &images,
                std::time::Duration::from_secs(120),
                std::time::Duration::from_secs(2),
            )?;
        }
        StepCommands::CheckGateway { plan } => {
            let plan = read_plan(plan)?;
            let allow_downgrade = std::env::var("SIMPLED_ALLOW_DOWNGRADE").is_ok_and(|v| v == "1");
            let lookup = |name: &str| std::env::var(name).ok();
            deploy_steps::check_gateway(&docker, &plan, &deploy_dir, &lookup, allow_downgrade)?;
        }
    }
    Ok(())
}

fn prepare_deployment_command(
    deployment_name: &str,
    bundle: &bundle_repo::BundleArgs,
    target_binary: Option<&Path>,
) -> Result<()> {
    // 1. Load specs
    let env_spec = spec_loader::load_env_spec(Path::new("."), Some(deployment_name), 0)?;

    let app_name = &env_spec.deployment(deployment_name)?.application.name;
    let bundle_path = bundle
        .locate(app_name, Path::new("."))?
        .context("Either --app-bundle or --download-bundle-from must be specified")?;

    let app_spec = spec_loader::load_app_spec(&bundle_path, Some(&env_spec))?;

    // 2. Validate
    validator::validate(&env_spec, &app_spec, deployment_name).context("Validation failed")?;

    println!("Validation passed for deployment {}", deployment_name);

    // 3. Resolve
    let resolved_spec = resolver::resolve(&env_spec, &app_spec, deployment_name).context("Resolution failed")?;

    // 4. Generate
    match env_spec.env_type {
        spec::DeploymentEnvType::K8S => {
            let output_dir = Path::new("manifests");
            k8s_generator::generate(&resolved_spec, output_dir).context("Generation failed")?;
            println!("Manifests generated in {:?}", output_dir);
        }
        spec::DeploymentEnvType::Docker(ref docker_spec) => {
            let output_dir = Path::new("docker-deploy");
            docker_generator::generate(&resolved_spec, docker_spec, output_dir).context("Generation failed")?;
            if docker_spec.swarm_mode {
                target_binary::provide(&output_dir.join("simpled"), docker_spec.target_platform, target_binary)
                    .context("Failed to ship the simpled binary that runs the deploy steps")?;
            }
            println!("Docker deployment script generated in {:?}", output_dir);
        }
        spec::DeploymentEnvType::Local => {
            bail!("prepare deployment doesn't support local deployments, use 'simpled local run' instead");
        }
    }

    Ok(())
}

/// Selects which deployment to act on for a local command.
///
/// - If `requested` is given, the matching deployment is returned (error if not found).
/// - Otherwise, when exactly one deployment is defined it is used.
/// - When multiple deployments are defined and none was requested, the user must
///   disambiguate with `--deployment`.
fn select_deployment<'a>(
    env_spec: &'a spec::DeploymentEnvironmentSpec,
    requested: Option<&str>,
) -> Result<&'a spec::DeploymentSpec> {
    if let Some(name) = requested {
        return env_spec.deployment(name);
    }

    match env_spec.deployments.as_slice() {
        [] => bail!("No deployments defined in envspec"),
        [single] => Ok(single),
        multiple => {
            let names = multiple.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ");
            bail!(
                "Multiple deployments defined ({}). Specify which one to run with --deployment <name>.",
                names
            )
        }
    }
}

fn local(command: &LocalCommands) -> Result<()> {
    let (root, deployment_name, instance_flag) = match command {
        LocalCommands::Run {
            path,
            deployment,
            instance,
            ..
        }
        | LocalCommands::OnlyExtra {
            path,
            deployment,
            instance,
            ..
        }
        | LocalCommands::GenerateConfig {
            path,
            deployment,
            instance,
            ..
        } => (
            path.as_ref().map(Path::new).unwrap_or(Path::new(".")),
            deployment,
            instance,
        ),
    };

    let instance = match ports::choose_instance(root, instance_flag.as_deref())? {
        ports::InstanceChoice::Fixed(n) => n,
        ports::InstanceChoice::Auto => bail!(
            "`auto` picks an instance only for `simpled test`; a local stack needs a fixed one so its URLs stay put"
        ),
    };

    let exclude = match command {
        LocalCommands::Run { exclude, .. } => exclude.clone().unwrap_or_default(),
        _ => Vec::new(),
    };
    let bundle = match command {
        LocalCommands::Run { bundle, .. } | LocalCommands::GenerateConfig { bundle, .. } => bundle.clone(),
        LocalCommands::OnlyExtra { .. } => bundle_repo::BundleArgs::default(),
    };

    let mut env_spec = spec_loader::load_env_spec(root, deployment_name.as_deref(), instance)?;
    let selected = select_deployment(&env_spec, deployment_name.as_deref())?.name.clone();
    let app_spec = spec_loader::load_local_app_spec(root, &mut env_spec, &selected, &bundle)?;

    let deployment = env_spec.deployment(&selected)?;

    validator::validate(&env_spec, &app_spec, &deployment.name).context("Validation failed")?;

    println!("Validation passed for deployment {}", deployment.name);

    // 3. Resolve
    let resolved_spec = resolver::resolve(&env_spec, &app_spec, &deployment.name).context("Resolution failed")?;
    if instance > 0 {
        println!(
            "Instance {}: {}",
            instance,
            run_local::ComposeTarget::local(&resolved_spec).describe()
        );
    }

    for excluded in &exclude {
        if !resolved_spec
            .current_deployment
            .services
            .iter()
            .any(|s| &s.full_name == excluded)
        {
            let mut names: Vec<&str> = resolved_spec
                .current_deployment
                .services
                .iter()
                .map(|s| s.full_name.as_str())
                .collect();
            names.sort();
            bail!(
                "--exclude names service '{}', which deployment '{}' does not have. Services: {}",
                excluded,
                deployment.name,
                names.join(", ")
            );
        }
    }

    // 4. Generate
    match env_spec.env_type {
        spec::DeploymentEnvType::K8S => return Err(anyhow!("Environment type should be local")),
        spec::DeploymentEnvType::Docker(_) => return Err(anyhow!("Environment type should be local")),
        spec::DeploymentEnvType::Local => match command {
            LocalCommands::GenerateConfig { .. } => {
                println!("Regenerating local configuration");
                run_local::generate_config(&resolved_spec)?;
            }
            LocalCommands::Run { bind, .. } | LocalCommands::OnlyExtra { bind, .. } => {
                // Kept for the life of the run: the gateway only stops when
                // `stop()` is called, but naming the handle says it is owned here.
                let _ingress = local_ingress::run(
                    resolved_spec.ingress.clone(),
                    &resolved_spec.current_deployment.name,
                    bind,
                    &run_local::ComposeTarget::local(&resolved_spec).dir,
                )?;
                match command {
                    LocalCommands::Run { .. } => {
                        println!("Running local deployment");
                        run_local::run(&resolved_spec, &exclude)?;
                    }
                    LocalCommands::OnlyExtra { .. } => {
                        println!("Running gateway and extra services only");
                        run_local::run_only_extra(&resolved_spec)?;
                    }
                    LocalCommands::GenerateConfig { .. } => unreachable!(),
                }
            }
        },
    }

    Ok(())
}
