use crate::deploy_plan::{escape, Plan, PlanJob};
use crate::docker_compose::{prepare_service, DockerCompose, DockerComposeNetwork, DockerService, ServiceNetwork};
use crate::docker_resources::DockerResources;
use crate::resolved_spec::{EnvironmentResolvedSpec, IngressResolvedSpec, LetsEncryptResolvedSpec};
use crate::secret_fetch::{self, create_executable, sh_quote, FetchScript};
use crate::spec::{AccessLogFormat, DockerSpecificSpec, SecretMount, ServiceVolumeType, VolumeBinding};
use crate::swarm_objects::{self, ObjectKind, SwarmObjects};
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

const DOCKER_NETWORK: &str = "common_network";
/// The standalone gateway's container.
pub const GATEWAY_CONTAINER: &str = "simpled-gateway";
/// Where simpled 2.0.x's Swarm gateway kept Traefik's ACME store, relative to
/// deploy.sh.
const LEGACY_ACME_JSON: &str = "letsencrypt/acme.json";
/// Let's Encrypt's production directory, the default of `letsencrypt.server`.
const LETSENCRYPT_DIRECTORY: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// The Traefik service the controller's routes and ACME challenges go to.
const TRAEFIK_CONTROLLER_SERVICE: &str = "simpled-controller";
const TRAEFIK_CONTROLLER_STRIP: &str = "strip-simpled-controller";
// Pinned, so two deploys of the same spec run the same gateway. A floating tag
// would change the ingress underneath a deployment that touched nothing else.
// Traefik stays on v2 because v3 changes the configuration format.
const TRAEFIK_IMAGE: &str = "traefik:v2.10";
const TRAEFIK_PING_ROUTER: &str = "simpled-ping";
const TRAEFIK_RESOLVER: &str = "myresolver";
/// Placeholder backend for redirect-only routers, which still have to name one.
const TRAEFIK_REDIRECT_SERVICE: &str = "redirect-noop";
/// The gateway's plan, relative to the deploy directory.
const INGRESS_PLAN: &str = "gateway/plan.json";
/// The gateway's stack file (Swarm), relative to the deploy directory.
pub const GATEWAY_STACK_FILE: &str = "gateway/docker-compose.yaml";
/// Stack file holding only the services the jobs depend on (phase 1 of a deploy).
const DEPS_COMPOSE_FILE: &str = "docker-compose.deps.yaml";
/// Default seconds to wait for one job to reach a terminal state.
const JOB_TIMEOUT_SECONDS: u32 = 600;
/// Seconds to wait for a dependency to report healthy in a standalone deploy.
const HEALTH_TIMEOUT_SECONDS: u32 = 300;

/// `wait_healthy` helper embedded in the generated standalone deploy script.
/// `docker run -d` returns once the container is created, so a declared
/// healthcheck is the only readiness signal a job's dependency can offer.
const WAIT_HEALTHY_FUNCTION: &str = r#"wait_healthy() {
  name="$1"
  echo "Waiting for ${name} to become healthy..."
  waited=0
  while [ "${waited}" -lt "${HEALTH_TIMEOUT}" ]; do
    status="$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' "${name}" 2>/dev/null || echo missing)"
    case "${status}" in
      healthy) return 0 ;;
      none) return 0 ;;
    esac
    sleep 2
    waited=$((waited + 2))
  done
  echo "Timed out after ${HEALTH_TIMEOUT}s waiting for ${name} to become healthy." >&2
  exit 1
}
"#;

/// The constraint that keeps a service on the one schedulable node. The stack
/// file carries it as written and `docker stack deploy` interpolates it from the
/// variable `SINGLE_NODE_CHECK` exports.
const NODE_PIN: &str = "node.id == ${SIMPLED_NODE_ID}";

/// Swarm with `multi_node: false`.
fn single_node(spec: &EnvironmentResolvedSpec) -> bool {
    matches!(&spec.env_type, crate::spec::DeploymentEnvType::Docker(d) if d.swarm_mode && !d.multi_node)
}

fn pin_to_deploy_node(service: &mut DockerService) {
    let deploy = service.deploy.get_or_insert_with(Default::default);
    let placement = deploy
        .placement
        .get_or_insert_with(|| crate::docker_compose::DockerPlacement {
            constraints: vec![],
            preferences: vec![],
        });
    if !placement.constraints.iter().any(|c| c == NODE_PIN) {
        placement.constraints.push(NODE_PIN.to_string());
    }
}

pub fn generate(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }

    if docker_spec.swarm_mode {
        generate_swarm(resolved_spec, output_dir)
    } else {
        generate_standalone(resolved_spec, output_dir)
    }
}

fn generate_standalone(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path) -> Result<()> {
    let deployment = &resolved_spec.current_deployment;

    // Create subdirs
    let configs_dir = output_dir.join("configs");
    fs::create_dir_all(&configs_dir)?;

    // 1. Configs. The resolver has already prefixed every config and secret name
    // with the application name, and that prefixed name is what the run commands
    // below mount back out of `./configs/` and `./secrets/`, so the names are used
    // verbatim here rather than prefixed a second time.
    for config in &deployment.configs {
        let cfg_dir = configs_dir.join(&config.name);
        fs::create_dir_all(&cfg_dir)?;
        for cfg_file in &config.files {
            let path = cfg_dir.join(&cfg_file.name);
            fs::write(&path, &cfg_file.content)?;
        }
    }

    // 2. Secrets. Ones with an `aws` source have no value yet — `fetch-secrets.sh`
    // writes their file on the deploy target.
    let secrets_dir = output_dir.join("secrets");
    fs::create_dir_all(&secrets_dir)?;
    let mut fetch_script = FetchScript::new();
    for secret in &deployment.secrets {
        match secret.deferred() {
            Some(reference) => fetch_script.fetch(secret, reference),
            None => fs::write(secrets_dir.join(&secret.name), secret.literal().unwrap_or_default())?,
        }
    }
    // Every lookup first, then the files: a secret that does not resolve stops the
    // script before any of the others has been written to the deploy target.
    for secret in &deployment.secrets {
        if secret.deferred().is_some() {
            fetch_script.write_to_file(secret, &format!("secrets/{}", secret.name));
        }
    }
    let controller_tokens =
        crate::controller::write_token_files(resolved_spec, output_dir, "controller", &mut fetch_script)?;
    if !fetch_script.is_empty() {
        fetch_script.write(
            &output_dir.join(secret_fetch::SCRIPT_NAME),
            "deploy.sh sources this script before it starts anything.",
        )?;
    }

    // 3. Envs
    let envs_dir = output_dir.join("envs");
    fs::create_dir_all(&envs_dir)?;

    // 4. Script
    let mut deploy_sh = create_executable(&output_dir.join("deploy.sh"))?;

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;
    write_deployed_at(&mut deploy_sh)?;

    write_fetch_secrets_call(&mut deploy_sh, &fetch_script)?;

    let network_name = DOCKER_NETWORK.to_string();
    writeln!(deploy_sh, "docker network create {} || true", network_name)?;

    // Jobs (migrations and other one-shot tasks) are ordered explicitly: first the
    // services they depend on, then the jobs themselves — run in the foreground so
    // a non-zero exit aborts the deploy — and only then the rest of the stack.
    let jobs = deployment.jobs_in_order();
    let prerequisites = deployment.job_prerequisites();
    let mut start_order: Vec<&crate::resolved_spec::ServiceResolvedSpec> = Vec::new();
    start_order.extend(prerequisites.iter().copied());
    start_order.extend(jobs.iter().copied());
    for service in deployment.long_running_services() {
        if !start_order.iter().any(|s| s.full_name == service.full_name) {
            start_order.push(service);
        }
    }

    if !jobs.is_empty() {
        writeln!(
            deploy_sh,
            "HEALTH_TIMEOUT=\"${{HEALTH_TIMEOUT:-{}}}\"",
            HEALTH_TIMEOUT_SECONDS
        )?;
        write!(deploy_sh, "{}", WAIT_HEALTHY_FUNCTION)?;
        writeln!(deploy_sh)?;
    }

    let mut jobs_started = false;
    for service in start_order {
        let is_job = matches!(service.service_type, crate::spec::ServiceType::Job);

        // Give the job dependencies a chance to become usable before the first
        // job starts. `docker run` returns as soon as the container is created,
        // so a declared healthcheck is the only readiness signal available.
        if is_job && !jobs_started {
            jobs_started = true;
            for dependency in &prerequisites {
                if dependency.healthcheck.as_ref().is_some_and(|hc| !hc.is_disabled()) {
                    writeln!(deploy_sh, "wait_healthy {}", dependency.full_name)?;
                }
            }
        }

        writeln!(deploy_sh, "echo 'Starting {}...'", service.full_name)?;
        writeln!(deploy_sh, "docker rm -f {} || true", service.full_name)?;

        // Create env file
        let env_file_name = format!("{}.env", service.full_name);
        let env_path = envs_dir.join(&env_file_name);
        let mut env_file = File::create(&env_path)?;

        // `docker run --env-file` takes each value literally up to the end of
        // the line, so a value with a line break cannot be carried.
        for env in &service.environment_variables {
            if env.value.contains(['\n', '\r']) {
                return Err(anyhow!(
                    "Environment variable {} of service {} contains a line break, which an env file \
                     cannot carry. Mount the value as a secret file instead.",
                    env.name,
                    service.full_name
                ));
            }
            writeln!(env_file, "{}={}", env.name, env.value)?;
        }

        // A job runs in the foreground: `set -e` then aborts the deploy when the
        // job exits non-zero, before any service that depends on its result
        // (e.g. a migrated schema) is started.
        let detach = if is_job { "" } else { "-d " };
        // The Traefik configuration addresses a service as `<deployment>_<service>`,
        // the name Swarm gives it. A standalone container only answers to its own
        // name, so it is given that alias too and both ingress types resolve it.
        write!(
            deploy_sh,
            "docker run {}--name {} --network {} --network-alias {}_{}",
            detach, service.full_name, network_name, deployment.name, service.full_name
        )?;

        for port in &service.ports {
            write!(deploy_sh, " -p {}:{}", port.external, port.internal)?;
        }

        // `expose` declares internal-only ports (reachable by other containers
        // on the network but not published to the host).
        for port in &service.expose {
            write!(deploy_sh, " --expose {}", port)?;
        }

        write!(deploy_sh, " --env-file $(pwd)/envs/{}", env_file_name)?;

        if crate::controller::model::labels::is_reported(service) {
            for (key, value) in crate::controller::model::labels::for_service(deployment, &service.full_name) {
                if key == crate::controller::model::labels::DEPLOYED_AT {
                    // Double quotes, so the shell expands the deploy time.
                    write!(deploy_sh, " --label \"{}={}\"", key, value)?;
                } else {
                    write!(deploy_sh, " --label {}", sh_quote(&format!("{}={}", key, value)))?;
                }
            }
        }

        // `docker run` has no CPU reservation; a CPU request is Swarm-only.
        if let Some(resources) =
            DockerResources::from_spec(&service.resources, &format!("Service '{}'", service.full_name))?
        {
            let (limits, reservations) = (resources.limits(), resources.reservations());
            if let Some(memory) = limits.memory {
                write!(deploy_sh, " --memory {}", memory)?;
            }
            if let Some(cpus) = limits.cpus {
                write!(deploy_sh, " --cpus {}", cpus)?;
            }
            if let Some(memory) = reservations.memory {
                write!(deploy_sh, " --memory-reservation {}", memory)?;
            }
        }

        for secret in &service.secrets {
            if let SecretMount::EnvVariable(var_name) = &secret.mount {
                // Quoted so a value with spaces or line breaks reaches the container whole.
                write!(deploy_sh, " -e {}=\"$(cat ./secrets/{})\"", var_name, secret.name)?;
            }
        }

        for config in &service.configs {
            let local_path = format!("$(pwd)/configs/{}", config.config_name);
            write!(deploy_sh, " -v {}:{}", local_path, config.mount_path)?;
        }

        for secret in &service.secrets {
            if let SecretMount::FilePath(path) = &secret.mount {
                let local_path = format!("$(pwd)/secrets/{}", secret.name);
                write!(deploy_sh, " -v {}:{}", local_path, path)?;
            }
        }

        // Healthcheck: map docker-compose `healthcheck` onto `docker run` flags.
        if let Some(hc) = &service.healthcheck {
            if hc.is_disabled() {
                write!(deploy_sh, " --no-healthcheck")?;
            } else {
                if let Some(cmd) = hc.health_cmd_string() {
                    write!(deploy_sh, " --health-cmd {}", sh_quote(&cmd))?;
                }
                if let Some(v) = &hc.interval {
                    write!(deploy_sh, " --health-interval {}", v)?;
                }
                if let Some(v) = &hc.timeout {
                    write!(deploy_sh, " --health-timeout {}", v)?;
                }
                if let Some(v) = hc.retries {
                    write!(deploy_sh, " --health-retries {}", v)?;
                }
                if let Some(v) = &hc.start_period {
                    write!(deploy_sh, " --health-start-period {}", v)?;
                }
            }
        }

        // `docker run --entrypoint` overrides only the executable, so the first
        // entrypoint token goes there and any remaining entrypoint tokens are
        // prepended to the container args after the image. The effective process
        // is therefore entrypoint ++ command, matching docker-compose.
        let mut trailing_args: Vec<String> = Vec::new();
        if let Some(entrypoint) = &service.entrypoint {
            let mut args = entrypoint.to_args();
            if !args.is_empty() {
                write!(deploy_sh, " --entrypoint {}", sh_quote(&args.remove(0)))?;
                trailing_args.extend(args);
            }
        }
        if let Some(command) = &service.command {
            trailing_args.extend(command.to_args());
        }

        // Quoted like the Swarm job invocation, so an argument with spaces or shell
        // metacharacters is passed to the container as one word.
        write!(deploy_sh, " {}", sh_quote(&service.image))?;
        for arg in trailing_args {
            write!(deploy_sh, " {}", sh_quote(&arg))?;
        }
        writeln!(deploy_sh)?;
    }

    // Containers of services the spec no longer has: `docker rm -f` above only
    // replaces the current ones, and a leftover would keep counting toward the
    // deployment's health.
    write_remove_retired_containers(&mut deploy_sh, deployment)?;

    // Ingress Container
    if !resolved_spec.ingress.rules.is_empty() {
        generate_traefik_standalone(
            resolved_spec,
            output_dir,
            &mut deploy_sh,
            network_name,
            &controller_tokens,
        )?;
    }

    Ok(())
}

/// Writes the loop that removes the deployment's labelled containers whose
/// `simpled.service` is not one of the services reported now.
fn write_remove_retired_containers(
    deploy_sh: &mut File,
    deployment: &crate::resolved_spec::DeploymentResolvedSpec,
) -> Result<()> {
    use crate::controller::model::labels;
    let current: Vec<String> = deployment
        .services
        .iter()
        .filter(|s| labels::is_reported(s))
        .map(|s| sh_quote(&s.full_name))
        .collect();
    // Both labels: the gateway carries `simpled.deployment` too, but no service.
    let filter = format!(
        "{} --filter {}",
        sh_quote(&format!("label={}={}", labels::DEPLOYMENT, deployment.name)),
        sh_quote(&format!("label={}", labels::SERVICE))
    );
    let format = sh_quote(&format!("{{{{.ID}}}} {{{{.Label \"{}\"}}}}", labels::SERVICE));
    writeln!(
        deploy_sh,
        "docker ps -a --filter {} --format {} | while read -r id service; do",
        filter, format
    )?;
    writeln!(deploy_sh, "  case \"$service\" in")?;
    if !current.is_empty() {
        writeln!(deploy_sh, "    {}) ;;", current.join("|"))?;
    }
    writeln!(
        deploy_sh,
        "    *) echo \"Removing the retired service $service...\"; docker rm -f \"$id\" >/dev/null ;;"
    )?;
    writeln!(deploy_sh, "  esac")?;
    writeln!(deploy_sh, "done")?;
    Ok(())
}

fn generate_swarm(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path) -> Result<()> {
    let deployment = &resolved_spec.current_deployment;

    // Application folder
    let app_dir = output_dir.join(&deployment.name);
    fs::create_dir_all(&app_dir)?;

    for volume in unbound_volumes(deployment) {
        let volume_sub_dir = app_dir.clone().join("volumes").join(volume);
        fs::create_dir_all(volume_sub_dir)?;
    }

    let network_name = DOCKER_NETWORK.to_string();

    // Every service (jobs included) is prepared, because that is what writes the
    // per-service `.env`, config and secret files a job needs at run time. Jobs
    // are kept out of the stack file itself: `docker stack deploy` has no
    // run-to-completion mode, so they are run separately by the deploy script.
    let mut prepared: HashMap<String, DockerService> = HashMap::new();

    // Secrets with an `aws` source are fetched by the script below, on the node,
    // rather than being written into the stack directory here. `prepare_service`
    // leaves their env values as `${SIMPLED_SECRET_*}` — which `docker stack
    // deploy` interpolates from the environment the deploy script exports — and
    // leaves the file behind their mounts to be created by the script.
    let mut fetch_script = FetchScript::new();
    for (secret, reference) in deployment.deferred_secrets() {
        fetch_script.fetch(secret, reference);
    }

    // Configs and file secrets become Swarm objects. The files they are created
    // from are written here, or, for a deferred secret, by the fetch script.
    let deployment_objects = swarm_objects::collect(deployment)?;
    for (path, content) in &deployment_objects.files {
        let path = output_dir.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
    }
    let mut fetched_files: Vec<&str> = Vec::new();

    for service in &deployment.services {
        let mut docker_service = prepare_service(service, resolved_spec, &app_dir)?;
        if let Some((secrets, configs)) = deployment_objects.mounts.get(&service.full_name) {
            docker_service.secrets = secrets.clone();
            docker_service.configs = configs.clone();
        }

        let mut networks = BTreeMap::new();
        networks.insert(
            "default".to_string(),
            ServiceNetwork {
                aliases: vec![service.full_name.clone()],
            },
        );

        docker_service.networks = networks;
        if single_node(resolved_spec) {
            pin_to_deploy_node(&mut docker_service);
        }

        // File-mounted deferred secrets: the fetch script writes the value to the
        // file its Swarm object is created from, before the objects are synced.
        for secret_option in &service.secrets {
            if !matches!(secret_option.mount, SecretMount::FilePath(_)) {
                continue;
            }
            let Some(secret) = deployment
                .secrets
                .iter()
                .find(|s| s.name == secret_option.name && s.deferred().is_some())
            else {
                continue;
            };
            if !fetched_files.contains(&secret.name.as_str()) {
                fetched_files.push(&secret.name);
                fetch_script.write_to_file(secret, &swarm_objects::secret_file(&deployment.name, &secret.name));
            }
        }

        prepared.insert(service.full_name.clone(), docker_service);
    }

    let controller_tokens =
        crate::controller::write_token_files(resolved_spec, output_dir, "gateway/controller", &mut fetch_script)?;
    if !fetch_script.is_empty() {
        fetch_script.write(
            &output_dir.join(secret_fetch::SCRIPT_NAME),
            "deploy.sh sources this script before it deploys the stack.",
        )?;
    }

    let jobs = deployment.jobs_in_order();
    let long_running = deployment.long_running_services();
    let prerequisites = deployment.job_prerequisites();

    let compose_network = || {
        let mut networks = BTreeMap::new();
        networks.insert(
            "default".to_string(),
            DockerComposeNetwork {
                external: true,
                name: network_name.clone(),
            },
        );
        networks
    };

    let stack_services: BTreeMap<String, DockerService> = long_running
        .iter()
        .filter_map(|s| prepared.get(&s.full_name).map(|d| (s.full_name.clone(), d.clone())))
        .collect();

    let compose = DockerCompose {
        name: None,
        services: stack_services,
        networks: compose_network(),
        volumes: crate::docker_compose::stack_volumes(resolved_spec),
        secrets: deployment_objects.objects.externals(ObjectKind::Secret),
        configs: deployment_objects.objects.externals(ObjectKind::Config),
    };

    let compose_path = &app_dir.join("docker-compose.yaml");
    let yaml = serde_yaml::to_string(&compose)?;
    fs::write(compose_path, yaml)?;

    // Phase-1 stack file: only the services the jobs need. Written when it is a
    // real subset — when the jobs need everything, phase 1 deploys the full stack
    // file instead and phase 3 has nothing left to do.
    let deps_only = !jobs.is_empty() && !prerequisites.is_empty() && prerequisites.len() < long_running.len();

    if deps_only {
        let dep_services: BTreeMap<String, DockerService> = prerequisites
            .iter()
            .filter_map(|s| prepared.get(&s.full_name).map(|d| (s.full_name.clone(), d.clone())))
            .collect();

        let deps_compose = DockerCompose {
            name: None,
            services: dep_services,
            networks: compose_network(),
            volumes: crate::docker_compose::stack_volumes(resolved_spec),
            secrets: deployment_objects.objects.externals(ObjectKind::Secret),
            configs: deployment_objects.objects.externals(ObjectKind::Config),
        };

        let deps_path = app_dir.join(DEPS_COMPOSE_FILE);
        fs::write(&deps_path, serde_yaml::to_string(&deps_compose)?)?;
    }

    // 4. Ingress Stack
    let ingress_dir = output_dir.join("gateway");
    fs::create_dir_all(&ingress_dir)?;

    let gateway_objects =
        generate_traefik_swarm(resolved_spec, &ingress_dir, network_name.clone(), &controller_tokens)?;

    // 5. Plans, which the deploy steps read.
    let plan_jobs = jobs
        .iter()
        .map(|job| {
            let docker_service = prepared
                .get(&job.full_name)
                .ok_or_else(|| anyhow!("Job {} was not prepared", job.full_name))?;
            Ok(PlanJob {
                name: job.full_name.clone(),
                service: format!("{}_{}", deployment.name, job.full_name),
                args: job_args(
                    &deployment.name,
                    &job.full_name,
                    docker_service,
                    &network_name,
                    &deployment.volume_bindings,
                )?,
                timeout_seconds: JOB_TIMEOUT_SECONDS,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let deployment_plan_path = format!("{}/plan.json", deployment.name);
    Plan {
        simpled_version: env!("CARGO_PKG_VERSION").to_string(),
        owner_label: deployment_objects.objects.owner_label.clone(),
        objects: deployment_objects.objects.plan_objects(),
        jobs: plan_jobs.clone(),
        tls: vec![],
    }
    .write(&output_dir.join(&deployment_plan_path))?;
    Plan {
        simpled_version: env!("CARGO_PKG_VERSION").to_string(),
        owner_label: gateway_objects.owner_label.clone(),
        objects: gateway_objects.plan_objects(),
        jobs: vec![],
        tls: tls_plan(resolved_spec),
    }
    .write(&output_dir.join(INGRESS_PLAN))?;

    // 6. Deploy Script
    let mut deploy_sh = create_executable(&output_dir.join("deploy.sh"))?;

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;
    writeln!(deploy_sh, "cd \"$(dirname \"$0\")\"")?;
    write_deployed_at(&mut deploy_sh)?;
    writeln!(deploy_sh)?;
    writeln!(
        deploy_sh,
        "# The steps below that would be a lot of shell are run by the simpled binary"
    )?;
    writeln!(
        deploy_sh,
        "# shipped next to this script, the version that prepared the deployment. They"
    )?;
    writeln!(
        deploy_sh,
        "# read <deployment>/plan.json and gateway/plan.json. A step that sets variables"
    )?;
    writeln!(
        deploy_sh,
        "# for the rest of the script prints `export` lines; they are captured first, so"
    )?;
    writeln!(
        deploy_sh,
        "# a failing step stops the script (`set -e`), and then evaluated."
    )?;
    writeln!(deploy_sh, "chmod +x ./simpled")?;
    writeln!(deploy_sh)?;
    writeln!(
        deploy_sh,
        "# One deploy at a time per swarm. The lock is released when this script exits,"
    )?;
    writeln!(deploy_sh, "# also when a step fails.")?;
    writeln!(
        deploy_sh,
        "lock=\"$(./simpled step lock {})\"",
        sh_quote(&deployment.name)
    )?;
    writeln!(deploy_sh, "eval \"${{lock}}\"")?;
    writeln!(deploy_sh, "trap './simpled step unlock' EXIT")?;
    writeln!(deploy_sh)?;
    write_fetch_secrets_call(&mut deploy_sh, &fetch_script)?;
    if single_node(resolved_spec) {
        writeln!(
            deploy_sh,
            "# multi_node: false keeps this environment on the one node that accepts tasks."
        )?;
        writeln!(deploy_sh, "node_pin=\"$(./simpled step check-nodes)\"")?;
        writeln!(deploy_sh, "eval \"${{node_pin}}\"")?;
        writeln!(deploy_sh)?;
    }
    writeln!(
        deploy_sh,
        "docker network create --driver overlay --attachable {} || true",
        network_name
    )?;

    // Disk is reclaimed *before* the rollout, not after it. At this point the
    // release currently serving traffic is running, so `prune -a` cannot remove
    // its image, and nothing prunes after the new release is pulled. The node
    // therefore always keeps the previous release next to the current one, and a
    // rollback never has to wait on the registry. Pruning is housekeeping, so a
    // failure here is reported but does not stop the deploy.
    writeln!(deploy_sh, "echo 'Pruning images no container uses...'")?;
    writeln!(
        deploy_sh,
        "docker image prune -af || echo 'Warning: image prune failed, continuing.' >&2"
    )?;

    // Bind-mounted volume directories are not created automatically on the node
    // during the first deployment, which makes `docker stack deploy` fail. Collect
    // every host directory the stack binds and ensure it exists (mkdir -p is a
    // no-op when the directory already exists). Paths are relative to the compose
    // file location, i.e. the deployment directory.
    let mut volume_dirs: Vec<String> = Vec::new();

    // Unbound named volumes are bound as `./volumes/<name>` inside the deployment
    // dir. A bound one is a Docker volume, or a host path handled below.
    for volume in unbound_volumes(deployment) {
        volume_dirs.push(format!("{}/volumes/{}", deployment.name, volume));
    }

    // Absolute host paths (e.g. `/var/lib/app_db:/var/lib/mysql`) have the same
    // problem: Swarm rejects the task outright when a bind source is missing,
    // and `docker stack deploy --detach=false` then waits on a service that can
    // never converge. They are kept apart from the relative ones because an
    // absolute path may already exist as something other than a directory —
    // `/var/run/docker.sock` is a socket — and `mkdir -p` fails on that, which
    // under `set -e` would abort the deploy. So an absolute path is only created
    // when nothing is there yet, the same thing `docker run -v` does.
    let mut absolute_dirs: Vec<String> = Vec::new();

    // Service-level path mounts: relative (`./data:/var/lib/...`) and absolute.
    for service in &deployment.services {
        for volume in &service.volumes {
            if let ServiceVolumeType::Path(from_path) = &volume.name {
                if let Some(rel) = from_path.strip_prefix("./") {
                    volume_dirs.push(format!("{}/{}", deployment.name, rel));
                } else if from_path.starts_with('/') {
                    absolute_dirs.push(from_path.clone());
                }
            }
        }
    }
    // A `path` binding is meant to exist on every node, such as a shared mount,
    // so creating it here creates it for all of them. A `node` + `path` one
    // lives on another node, where this script cannot create it.
    for binding in deployment.volume_bindings.values() {
        if let VolumeBinding::Path { path } = binding {
            absolute_dirs.push(path.clone());
        }
    }

    volume_dirs.sort();
    volume_dirs.dedup();
    absolute_dirs.sort();
    absolute_dirs.dedup();

    if !volume_dirs.is_empty() || !absolute_dirs.is_empty() {
        writeln!(deploy_sh, "echo 'Ensuring volume directories exist...'")?;
        for dir in &volume_dirs {
            writeln!(deploy_sh, "mkdir -p \"{}\"", dir)?;
        }
        for dir in &absolute_dirs {
            writeln!(deploy_sh, "[ -e \"{0}\" ] || mkdir -p \"{0}\"", dir)?;
        }
    }

    // Every Swarm object the stacks and jobs mount exists before anything is
    // deployed. Their names are exported for the stack files to interpolate.
    writeln!(deploy_sh)?;
    writeln!(deploy_sh, "echo 'Syncing configs and secrets...'")?;
    for (var, plan) in [
        ("gateway_objects", INGRESS_PLAN.to_string()),
        ("objects", deployment_plan_path.clone()),
    ] {
        writeln!(deploy_sh, "{}=\"$(./simpled step sync-objects {})\"", var, plan)?;
        writeln!(deploy_sh, "eval \"${{{}}}\"", var)?;
    }
    writeln!(deploy_sh)?;

    if !tls_plan(resolved_spec).is_empty() {
        // simpled 2.0.x let Traefik keep its certificates in letsencrypt/; they
        // become secrets once, so the upgrade requests nothing new.
        writeln!(deploy_sh, "if [ -f {} ]; then", LEGACY_ACME_JSON)?;
        writeln!(
            deploy_sh,
            "  ./simpled step tls-import {} {}",
            INGRESS_PLAN, LEGACY_ACME_JSON
        )?;
        writeln!(deploy_sh, "fi")?;
        writeln!(deploy_sh, "tls=\"$(./simpled step tls-secrets {})\"", INGRESS_PLAN)?;
        writeln!(deploy_sh, "eval \"${{tls}}\"")?;
    }
    // Every deployment of the env spec redeploys the shared gateway from its own
    // copy of the env spec, so what runs is checked first.
    writeln!(deploy_sh, "./simpled step check-gateway {}", INGRESS_PLAN)?;
    // A 2.0.x gateway in the `ingress` stack holds the ports; removed once.
    writeln!(deploy_sh, "./simpled step retire-legacy-stack")?;
    // --prune removes services that still carry the stack label but are no
    // longer in the file, e.g. after the ingress type was switched.
    writeln!(
        deploy_sh,
        "docker stack deploy -c {} {} --prune --detach=false",
        GATEWAY_STACK_FILE,
        crate::deploy_steps::STACK
    )?;

    if jobs.is_empty() {
        // --prune removes services that are still labelled as part of this stack
        // but are no longer in the stack file, i.e. services deleted from the
        // spec since the last deploy. Only ever passed when the file being
        // deployed is the complete stack file.
        // --detach=false blocks until every task is running (and healthy, when
        // the service declares a healthcheck), so a rollout that fails to
        // converge fails the deploy instead of being reported as a success.
        writeln!(
            deploy_sh,
            "docker stack deploy -c {}/docker-compose.yaml {} --with-registry-auth --prune --detach=false",
            deployment.name, deployment.name
        )?;
    } else {
        // Jobs are run between two partial rollouts of the same stack. `docker
        // stack deploy` only removes services missing from the file when it is
        // given --prune, so deploying a subset first and the full file afterwards
        // is an additive, idempotent rollout: phase 3 reports the phase-1
        // services as up to date and leaves them running. That also means --prune
        // must never be passed for a partial file — it would tear down every
        // service phase 3 is about to bring back.
        writeln!(deploy_sh)?;
        writeln!(
            deploy_sh,
            "echo '== Phase 1/3: starting services the jobs depend on =='"
        )?;
        if prerequisites.is_empty() {
            writeln!(
                deploy_sh,
                "echo 'No job dependencies declared, nothing to start first.'"
            )?;
        } else {
            let phase1_file = if deps_only {
                DEPS_COMPOSE_FILE
            } else {
                "docker-compose.yaml"
            };
            // Only the full stack file may prune; the deps-only subset may not.
            let prune = if deps_only { "" } else { " --prune" };
            // --detach=false blocks until every service in the file has converged,
            // i.e. its tasks are running and (when a healthcheck is declared)
            // healthy. That is what makes it safe to run the jobs next.
            writeln!(
                deploy_sh,
                "docker stack deploy -c {}/{} {} --with-registry-auth{} --detach=false",
                deployment.name, phase1_file, deployment.name, prune
            )?;
        }

        writeln!(deploy_sh)?;
        writeln!(deploy_sh, "echo '== Phase 2/3: running jobs =='")?;
        for job in &plan_jobs {
            writeln!(
                deploy_sh,
                "./simpled step run-job {} {}",
                deployment_plan_path,
                sh_quote(&job.name)
            )?;
        }

        writeln!(deploy_sh)?;
        writeln!(deploy_sh, "echo '== Phase 3/3: deploying the rest of the stack =='")?;
        if long_running.is_empty() {
            writeln!(deploy_sh, "echo 'This deployment has no long-running services.'")?;
        } else if deps_only || prerequisites.is_empty() {
            writeln!(
                deploy_sh,
                "docker stack deploy -c {}/docker-compose.yaml {} --with-registry-auth --prune --detach=false",
                deployment.name, deployment.name
            )?;
        } else {
            // Phase 1 already deployed every long-running service.
            writeln!(
                deploy_sh,
                "echo 'All services were started in phase 1, nothing left to deploy.'"
            )?;
        }
    }

    // Only reached when everything above succeeded (`set -e`).
    writeln!(deploy_sh)?;
    writeln!(
        deploy_sh,
        "echo 'Removing configs and secrets older than the previous deploy...'"
    )?;
    writeln!(deploy_sh, "./simpled step prune-objects {}", INGRESS_PLAN)?;
    writeln!(deploy_sh, "./simpled step prune-objects {}", deployment_plan_path)?;

    Ok(())
}

/// Have the deploy script source `fetch-secrets.sh` before it does anything else.
/// Sourcing rather than running it is what makes the fetched values visible to
/// the rest of the script, and to the `${...}` interpolation `docker stack
/// deploy` performs on the stack file.
fn write_fetch_secrets_call(deploy_sh: &mut File, fetch_script: &FetchScript) -> Result<()> {
    if fetch_script.is_empty() {
        return Ok(());
    }
    writeln!(
        deploy_sh,
        "# Read the secrets with an `aws` source from AWS Secrets Manager. Sourced, so"
    )?;
    writeln!(deploy_sh, "# the values it exports are visible to the commands below.")?;
    writeln!(deploy_sh, ". ./{}", secret_fetch::SCRIPT_NAME)?;
    writeln!(deploy_sh)?;
    Ok(())
}

/// The `docker service create` arguments of one job, after the flags every job
/// gets: a translation of the compose representation of the service. Everything
/// a job needs is covered: env file, inline environment (including env-variable
/// secrets), configs and file secrets, volumes, placement, resources, entrypoint
/// and command. Ports and healthchecks are not translated; neither means anything
/// for a task that is expected to exit.
///
/// Every argument is a template (`deploy_plan`): the inline environment already
/// is one, as compose reads it from the stack file too, and every literal is
/// escaped. `${DEPLOY_DIR}` is the deploy directory, the object variables are the
/// ones `step sync-objects` exported.
fn job_args(
    deployment_name: &str,
    job_name: &str,
    service: &DockerService,
    network_name: &str,
    volume_bindings: &std::collections::BTreeMap<String, VolumeBinding>,
) -> Result<Vec<String>> {
    let mut args: Vec<String> = vec!["--network".into(), escape(network_name)];

    for env_file in &service.env_file {
        args.push("--env-file".into());
        args.push(node_path(deployment_name, env_file));
    }

    for (name, value) in &service.environment {
        args.push("-e".into());
        args.push(format!("{}={}", escape(name), value));
    }

    if let Some(resources) = service.deploy.as_ref().and_then(|d| d.resources.as_ref()) {
        let (limits, reservations) = (resources.limits(), resources.reservations());
        let flags = [
            ("--limit-memory", limits.memory),
            ("--limit-cpu", limits.cpus),
            ("--reserve-memory", reservations.memory),
            ("--reserve-cpu", reservations.cpus),
        ];
        for (flag, value) in flags {
            if let Some(value) = value {
                args.push(flag.into());
                args.push(escape(&value));
            }
        }
    }

    if let Some(placement) = service.deploy.as_ref().and_then(|d| d.placement.as_ref()) {
        for constraint in &placement.constraints {
            args.push("--constraint".into());
            // The node pin names the variable `step check-nodes` exported.
            args.push(if constraint == NODE_PIN {
                constraint.clone()
            } else {
                escape(constraint)
            });
        }
        for preference in &placement.preferences {
            args.push("--placement-pref".into());
            args.push(escape(&format!("spread={}", preference.spread)));
        }
    }

    for (flag, mounts) in [("--secret", &service.secrets), ("--config", &service.configs)] {
        for object in mounts {
            args.push(flag.into());
            args.push(format!(
                "source=${{{}}},target={},mode={:o}",
                object.var,
                escape(&object.target),
                object.mode
            ));
        }
    }

    for volume in &service.volumes {
        // Compose volume entries are `<source>:<target>`; the source is relative
        // to the stack file, i.e. to the deployment directory.
        let Some((source, target)) = volume.split_once(':') else {
            return Err(anyhow!("Job {} has an invalid volume entry '{}'", job_name, volume));
        };
        args.push("--mount".into());
        // A Docker volume the stack declares: `docker stack deploy` names it
        // `<stack>_<name>`, so the job mounts that same volume.
        if let Some(docker_volume) = volume_bindings
            .get(source)
            .and_then(crate::docker_compose::DockerVolume::for_binding)
        {
            let mut mount = format!("type=volume,source={}_{},target={}", deployment_name, source, target);
            if let Some(driver) = &docker_volume.driver {
                mount.push_str(&format!(",volume-driver={}", driver));
            }
            for (key, value) in &docker_volume.driver_opts {
                // `--mount` is CSV, so an option whose value has a comma is quoted.
                let opt = format!("volume-opt={}={}", key, value);
                if opt.contains(',') {
                    mount.push_str(&format!(",\"{}\"", opt));
                } else {
                    mount.push_str(&format!(",{}", opt));
                }
            }
            args.push(escape(&mount));
            continue;
        }
        args.push(format!(
            "type=bind,source={},target={}",
            node_path(deployment_name, source),
            escape(target)
        ));
    }

    // `docker service create --entrypoint` overrides only the executable, so any
    // remaining entrypoint tokens are prepended to the container args, the same
    // split `docker run` needs. Requires a Docker CLI that supports the flag
    // (25.0+); jobs that do not override the entrypoint work on any version.
    let mut trailing_args: Vec<String> = Vec::new();
    if let Some(entrypoint) = &service.entrypoint {
        let mut entry = entrypoint.to_args();
        if !entry.is_empty() {
            args.push("--entrypoint".into());
            args.push(escape(&entry.remove(0)));
            trailing_args.extend(entry);
        }
    }
    if let Some(command) = &service.command {
        trailing_args.extend(command.to_args());
    }

    args.push(escape(&service.image));
    args.extend(trailing_args.iter().map(|a| escape(a)));
    Ok(args)
}

/// Absolute path on the deployment node for a path written relative to the stack
/// file. Already-absolute paths (a user-declared host mount) are left alone.
fn node_path(deployment_name: &str, path: &str) -> String {
    if path.starts_with('/') {
        escape(path)
    } else {
        format!(
            "${{DEPLOY_DIR}}/{}/{}",
            escape(deployment_name),
            escape(path.trim_start_matches("./"))
        )
    }
}

fn generate_traefik_standalone(
    resolved_spec: &EnvironmentResolvedSpec,
    output_dir: &Path,
    deploy_sh: &mut File,
    network_name: String,
    controller_tokens: &[&str],
) -> Result<()> {
    let traefik_dir = output_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    let has_tls = resolved_spec.ingress.terminates_tls();
    let letsencrypt = resolved_spec.ingress.tls.as_ref().and_then(|t| t.letsencrypt.as_ref());

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    write_traefik_static_config(
        &mut static_conf,
        has_tls,
        letsencrypt,
        &resolved_spec.ingress.trusted_proxies,
        resolved_spec.ingress.tls_upstream(),
        resolved_spec.ingress.access_log,
    )?;

    // Standalone keeps Traefik as its own ACME client, on its single host.
    generate_traefik_dynamic_config(&resolved_spec.ingress, &traefik_dir.join("dynamic_conf.yml"), &[])?;

    write_standalone_controller(resolved_spec, output_dir, deploy_sh, controller_tokens)?;

    writeln!(deploy_sh, "echo 'Starting Traefik ingress...'")?;
    // `traefik-ingress` is the name before simpled 2.1; it holds the ports too.
    writeln!(deploy_sh, "docker rm -f traefik-ingress {} || true", GATEWAY_CONTAINER)?;

    write!(
        deploy_sh,
        "docker run -d --name {} --network {}",
        GATEWAY_CONTAINER, network_name
    )?;
    write!(deploy_sh, " -p 80:80")?;
    if has_tls {
        write!(deploy_sh, " -p 443:443")?;
    }

    write!(deploy_sh, " -v $(pwd)/traefik/traefik.yml:/etc/traefik/traefik.yml")?;
    write!(
        deploy_sh,
        " -v $(pwd)/traefik/dynamic_conf.yml:/etc/traefik/dynamic_conf.yml"
    )?;

    if letsencrypt.is_some() {
        let le_dir = output_dir.join("letsencrypt");
        fs::create_dir_all(&le_dir)?;
        write!(deploy_sh, " -v $(pwd)/letsencrypt:/letsencrypt")?;
    }

    write!(deploy_sh, " -e DEPLOY_DATE=$(date +%s)")?;
    writeln!(deploy_sh, " {}", TRAEFIK_IMAGE)?;
    // `docker run` takes one network; the controller's is joined after the start.
    writeln!(
        deploy_sh,
        "docker network connect {} {}",
        crate::controller::CONTROL_NETWORK,
        GATEWAY_CONTAINER
    )?;

    Ok(())
}

/// The controller's container, before the gateway that routes to it. Not root: on
/// standalone Docker it runs on the host that runs deploy.sh, so the group that
/// owns the socket is looked up here and added.
fn write_standalone_controller(
    resolved_spec: &EnvironmentResolvedSpec,
    output_dir: &Path,
    deploy_sh: &mut File,
    tokens: &[&str],
) -> Result<()> {
    crate::controller::write_config(
        &output_dir.join("controller"),
        &crate::controller::config_for(resolved_spec, crate::controller::source::Mode::Standalone),
    )?;
    let network = crate::controller::CONTROL_NETWORK;
    writeln!(deploy_sh, "echo 'Starting the simpled controller...'")?;
    writeln!(deploy_sh, "docker network create {} || true", network)?;
    writeln!(deploy_sh, "docker rm -f simpled-controller || true")?;
    writeln!(
        deploy_sh,
        "docker run -d --name simpled-controller --network {} --network-alias {} --restart unless-stopped \\",
        network,
        crate::controller::HOST
    )?;
    writeln!(
        deploy_sh,
        "  --read-only --cap-drop ALL --group-add \"$(stat -c %g /var/run/docker.sock)\" \\"
    )?;
    writeln!(
        deploy_sh,
        "  -v /var/run/docker.sock:/var/run/docker.sock -v $(pwd)/controller/controller.json:{}:ro \\",
        crate::controller::CONFIG_PATH
    )?;
    for token in tokens {
        writeln!(
            deploy_sh,
            "  -v $(pwd)/{}:{}:ro \\",
            crate::controller::token_file("controller", token),
            crate::controller::token_path(token)
        )?;
    }
    writeln!(deploy_sh, "  \"{}\"", crate::controller::image_reference())?;
    Ok(())
}

/// The time of this deploy, which every service's `simpled.deployed_at` label
/// carries. Exported, because `docker stack deploy` reads it from the
/// environment.
fn write_deployed_at(deploy_sh: &mut File) -> Result<()> {
    writeln!(
        deploy_sh,
        "export {}=\"$(date +%s)\"",
        crate::controller::model::labels::DEPLOYED_AT_VAR
    )?;
    Ok(())
}

/// Returns the gateway's Swarm objects, which the deploy script syncs.
fn generate_traefik_swarm(
    resolved_spec: &EnvironmentResolvedSpec,
    ingress_dir: &Path,
    network_name: String,
    controller_tokens: &[&str],
) -> Result<SwarmObjects> {
    let traefik_dir = ingress_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    // Reuse generate logic for config, but write to new dir
    let has_tls = resolved_spec.ingress.terminates_tls();
    let upstream = resolved_spec.ingress.tls_upstream();
    // Traefik runs no ACME on Swarm: it reads each domain's certificate from a
    // Swarm secret, which the controller issues and renews.
    let cert_domains = crate::gateway_tls::domains(&resolved_spec.ingress);

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    write_traefik_static_config(
        &mut static_conf,
        has_tls,
        None,
        &resolved_spec.ingress.trusted_proxies,
        upstream,
        resolved_spec.ingress.access_log,
    )?;

    generate_traefik_dynamic_config(
        &resolved_spec.ingress,
        &traefik_dir.join("dynamic_conf.yml"),
        &cert_domains,
    )?;

    let mut stack = File::create(ingress_dir.join("docker-compose.yaml"))?;
    writeln!(stack, "version: '3.8'")?;
    writeln!(stack, "services:")?;
    writeln!(stack, "  gateway:")?;
    writeln!(stack, "    image: {}", TRAEFIK_IMAGE)?;
    write_swarm_ingress_deploy(
        &mut stack,
        &resolved_spec.ingress,
        single_node(resolved_spec),
        &resolved_spec.current_deployment.name,
    )?;
    write_swarm_host_ports(&mut stack, has_tls)?;

    // The gateway's own files are Swarm configs, owned by the environment rather
    // than by a deployment, so every node running a gateway task has them.
    let mut objects = SwarmObjects::new("simpled.scope=environment".to_string());
    writeln!(stack, "    configs:")?;
    for (file, target) in [
        ("traefik.yml", "/etc/traefik/traefik.yml"),
        ("dynamic_conf.yml", "/etc/traefik/dynamic_conf.yml"),
    ] {
        let object = objects.add(
            ObjectKind::Config,
            &format!("gateway_{}", file),
            &format!("gateway/traefik/{}", file),
            "SIMPLED_GATEWAY_OBJECT_",
        );
        writeln!(stack, "      - source: {}", object.base)?;
        writeln!(stack, "        target: {}", target)?;
        writeln!(stack, "        mode: 0444")?;
    }

    // One certificate secret per domain. Which one is current is looked up by
    // `step tls-secrets` when deploy.sh runs, so a redeploy keeps the one the
    // controller last rolled out.
    if !cert_domains.is_empty() {
        writeln!(stack, "    secrets:")?;
        for (i, domain) in cert_domains.iter().enumerate() {
            writeln!(stack, "      - source: tls_{}", i + 1)?;
            writeln!(stack, "        target: {}", crate::gateway_tls::cert_path(domain))?;
            writeln!(stack, "        mode: 0400")?;
        }
    }
    writeln!(stack, "    networks:")?;
    writeln!(stack, "      default:")?;
    writeln!(stack, "      {}:", crate::controller::CONTROL_NETWORK)?;

    write_swarm_controller(&mut stack, resolved_spec, ingress_dir, &mut objects, controller_tokens)?;

    writeln!(stack, "networks:")?;
    writeln!(stack, "  default:")?;
    writeln!(stack, "    external: true")?;
    writeln!(stack, "    name: {}", network_name)?;
    // Created by the stack. Not attachable, so only these two services join it.
    writeln!(stack, "  {}:", crate::controller::CONTROL_NETWORK)?;
    writeln!(stack, "    name: {}", crate::controller::CONTROL_NETWORK)?;
    writeln!(stack, "    driver: overlay")?;

    writeln!(stack, "configs:")?;
    for (key, external) in objects.externals(ObjectKind::Config) {
        writeln!(stack, "  {}:", key)?;
        writeln!(stack, "    external: true")?;
        writeln!(stack, "    name: \"{}\"", external.name)?;
    }
    let token_secrets: Vec<_> = objects.externals(ObjectKind::Secret).into_iter().collect();
    if !cert_domains.is_empty() || !token_secrets.is_empty() {
        writeln!(stack, "secrets:")?;
        for (key, external) in &token_secrets {
            writeln!(stack, "  {}:", key)?;
            writeln!(stack, "    external: true")?;
            writeln!(stack, "    name: \"{}\"", external.name)?;
        }
    }
    if !cert_domains.is_empty() {
        for (i, tls) in tls_plan(resolved_spec).iter().enumerate() {
            writeln!(stack, "  tls_{}:", i + 1)?;
            writeln!(stack, "    external: true")?;
            writeln!(stack, "    name: \"${{{}}}\"", tls.var)?;
        }
    }

    Ok(objects)
}

/// The gateway's certificate secrets, in the order the stack numbers them.
fn tls_plan(resolved_spec: &EnvironmentResolvedSpec) -> Vec<crate::deploy_plan::PlanTls> {
    crate::gateway_tls::domains(&resolved_spec.ingress)
        .into_iter()
        .enumerate()
        .map(|(i, domain)| crate::deploy_plan::PlanTls {
            domain,
            var: format!("{}{}", crate::gateway_tls::VAR_PREFIX, i + 1),
        })
        .collect()
}

fn write_traefik_static_config(
    file: &mut File,
    has_tls: bool,
    letsencrypt: Option<&LetsEncryptResolvedSpec>,
    trusted_proxies: &[String],
    ping: bool,
    access_log: Option<AccessLogFormat>,
) -> Result<()> {
    writeln!(file, "entryPoints:")?;
    writeln!(file, "  web:")?;
    writeln!(file, "    address: \":80\"")?;
    write_traefik_trusted_ips(file, trusted_proxies)?;
    if has_tls {
        writeln!(file, "    http:")?;
        writeln!(file, "      redirections:")?;
        writeln!(file, "        entryPoint:")?;
        writeln!(file, "          to: websecure")?;
        writeln!(file, "          scheme: https")?;
        // Without Traefik's own ACME (Swarm, where the controller issues the
        // certificates): below every router on `web`, so the challenge route is
        // answered over plain HTTP instead of redirected. Traefik's default is
        // above every router.
        if letsencrypt.is_none() {
            writeln!(file, "          priority: 1")?;
        }
        writeln!(file, "  websecure:")?;
        writeln!(file, "    address: \":443\"")?;
        write_traefik_trusted_ips(file, trusted_proxies)?;
    }
    writeln!(file, "providers:")?;
    writeln!(file, "  file:")?;
    writeln!(file, "    filename: \"/etc/traefik/dynamic_conf.yml\"")?;
    writeln!(file, "    watch: true")?;
    // Traefik's own health answer on the entrypoint the load balancer reaches.
    // Its built-in router is `PathPrefix(`/ping`)`, which would also swallow
    // `/pingback` on every domain, so it is routed by hand instead: an exact
    // `Path(`/ping`)` router in the dynamic config (TRAEFIK_PING_ROUTER).
    if ping {
        writeln!(file, "ping:")?;
        writeln!(file, "  entryPoint: web")?;
        writeln!(file, "  manualRouting: true")?;
    }
    write_traefik_access_log(file, access_log)?;
    if let Some(le) = letsencrypt {
        writeln!(file, "certificatesResolvers:")?;
        writeln!(file, "  {}:", TRAEFIK_RESOLVER)?;
        writeln!(file, "    acme:")?;
        writeln!(file, "      email: \"{}\"", le.email)?;
        // Only when it is not Let's Encrypt's, Traefik's default, so a spec
        // without `server` keeps the config it had.
        if le.server != LETSENCRYPT_DIRECTORY {
            writeln!(file, "      caServer: {}", serde_json::to_string(&le.server)?)?;
        }
        writeln!(file, "      storage: \"/letsencrypt/acme.json\"")?;
        writeln!(file, "      httpChallenge:")?;
        writeln!(file, "        entryPoint: web")?;
    }
    Ok(())
}

/// `accessLog`, on stdout beside Traefik's own log, so `docker service logs`
/// and any log driver or agent pick it up. The client IP (`ClientHost`) is the
/// peer's, or the X-Forwarded-For of a `trusted_proxies` peer, which Traefik
/// strips from any other; JSON also keeps that header, every other one dropped.
fn write_traefik_access_log(file: &mut File, access_log: Option<AccessLogFormat>) -> Result<()> {
    let Some(format) = access_log else {
        return Ok(());
    };
    writeln!(file, "accessLog:")?;
    match format {
        AccessLogFormat::Common => writeln!(file, "  format: common")?,
        AccessLogFormat::Json => {
            writeln!(file, "  format: json")?;
            writeln!(file, "  fields:")?;
            writeln!(file, "    headers:")?;
            writeln!(file, "      defaultMode: drop")?;
            writeln!(file, "      names:")?;
            writeln!(file, "        X-Forwarded-For: keep")?;
        }
    }
    Ok(())
}

/// Host-mode publishing: the routing mesh would SNAT every client to a mesh
/// address. The ports bind only on the node running the gateway task, and the
/// default stop-first update order keeps two tasks from fighting over them.
fn write_traefik_trusted_ips(file: &mut File, trusted_proxies: &[String]) -> Result<()> {
    if trusted_proxies.is_empty() {
        return Ok(());
    }
    writeln!(file, "    forwardedHeaders:")?;
    writeln!(file, "      trustedIPs:")?;
    for entry in trusted_proxies {
        writeln!(file, "        - \"{}\"", entry)?;
    }
    Ok(())
}

/// `deploy:` of the gateway service. Its labels name the simpled version and the
/// deployment that deployed it, which `step check-gateway` reads back on the next
/// deploy. Service labels change no task, so they restart nothing.
fn write_swarm_ingress_deploy(
    stack: &mut File,
    ingress: &IngressResolvedSpec,
    pin: bool,
    deployment: &str,
) -> Result<()> {
    writeln!(stack, "    deploy:")?;
    writeln!(stack, "      labels:")?;
    writeln!(
        stack,
        "        simpled.version: {}",
        serde_json::to_string(env!("CARGO_PKG_VERSION"))?
    )?;
    writeln!(
        stack,
        "        simpled.deployment: {}",
        serde_json::to_string(deployment)?
    )?;
    if ingress.global {
        writeln!(stack, "      mode: global")?;
    }
    let mut constraints: Vec<&str> = ingress
        .placement
        .iter()
        .flat_map(|p| p.constraints.iter().map(String::as_str))
        .collect();
    if pin {
        constraints.push(NODE_PIN);
    }
    let spread: &[String] = ingress.placement.as_ref().map(|p| p.spread.as_slice()).unwrap_or(&[]);
    if !constraints.is_empty() || !spread.is_empty() {
        writeln!(stack, "      placement:")?;
        if !constraints.is_empty() {
            writeln!(stack, "        constraints:")?;
            for constraint in constraints {
                writeln!(stack, "          - {}", serde_json::to_string(constraint)?)?;
            }
        }
        if !spread.is_empty() {
            writeln!(stack, "        preferences:")?;
            for spread in spread {
                writeln!(stack, "          - spread: {}", serde_json::to_string(spread)?)?;
            }
        }
    }
    Ok(())
}

/// Named volumes the env spec does not bind: each is a directory under
/// `<deployment>/volumes/`, as in simpled 1.x.
fn unbound_volumes(deployment: &crate::resolved_spec::DeploymentResolvedSpec) -> impl Iterator<Item = &String> {
    deployment
        .volumes
        .iter()
        .filter(|v| !deployment.volume_bindings.contains_key(*v))
}

fn write_swarm_host_ports(stack: &mut File, has_tls: bool) -> Result<()> {
    writeln!(stack, "    ports:")?;
    let ports: &[u16] = if has_tls { &[80, 443] } else { &[80] };
    for port in ports {
        writeln!(stack, "      - target: {}", port)?;
        writeln!(stack, "        published: {}", port)?;
        writeln!(stack, "        protocol: tcp")?;
        writeln!(stack, "        mode: host")?;
    }
    Ok(())
}

/// No DEPLOY_DATE here: the gateway restarts when its configs or image change,
/// not on every deploy of every deployment that shares it.
/// The `controller` service of the ingress stack, next to the gateway. It reads the
/// swarm through the Docker socket, so it runs on a manager. As root, because it
/// may land on a manager other than the one running deploy.sh, whose `docker`
/// group need not have the same number; with the socket it could act as root
/// anyway. Its filesystem is read-only and it has no capabilities.
fn write_swarm_controller(
    stack: &mut File,
    resolved_spec: &EnvironmentResolvedSpec,
    ingress_dir: &Path,
    objects: &mut SwarmObjects,
    tokens: &[&str],
) -> Result<()> {
    crate::controller::write_config(
        &ingress_dir.join("controller"),
        &crate::controller::config_for(resolved_spec, crate::controller::source::Mode::Swarm),
    )?;
    let config = objects.add(
        ObjectKind::Config,
        "gateway_controller.json",
        "gateway/controller/controller.json",
        "SIMPLED_GATEWAY_OBJECT_",
    );

    writeln!(stack, "  controller:")?;
    writeln!(
        stack,
        "    image: {}",
        serde_json::to_string(&crate::controller::image_reference())?
    )?;
    writeln!(stack, "    user: \"0:0\"")?;
    writeln!(stack, "    read_only: true")?;
    writeln!(stack, "    cap_drop:")?;
    writeln!(stack, "      - ALL")?;
    writeln!(stack, "    volumes:")?;
    writeln!(stack, "      - /var/run/docker.sock:/var/run/docker.sock")?;
    writeln!(stack, "    configs:")?;
    writeln!(stack, "      - source: {}", config.base)?;
    writeln!(stack, "        target: {}", crate::controller::CONFIG_PATH)?;
    writeln!(stack, "        mode: 0444")?;
    // The private API's tokens: content-addressed gateway secrets, so every
    // manager the controller may run on has them.
    if !tokens.is_empty() {
        writeln!(stack, "    secrets:")?;
        for token in tokens {
            let secret = objects.add(
                ObjectKind::Secret,
                &format!("gateway_controller_{}", token),
                &crate::controller::token_file("gateway/controller", token),
                "SIMPLED_GATEWAY_OBJECT_",
            );
            writeln!(stack, "      - source: {}", secret.base)?;
            writeln!(stack, "        target: {}", crate::controller::token_path(token))?;
            writeln!(stack, "        mode: 0400")?;
        }
    }
    writeln!(stack, "    networks:")?;
    writeln!(stack, "      {}:", crate::controller::CONTROL_NETWORK)?;
    writeln!(stack, "    deploy:")?;
    writeln!(stack, "      placement:")?;
    writeln!(stack, "        constraints:")?;
    writeln!(stack, "          - \"node.role == manager\"")?;
    if single_node(resolved_spec) {
        writeln!(stack, "          - {}", serde_json::to_string(NODE_PIN)?)?;
    }
    Ok(())
}

/// `cert_domains`: the domains whose certificates Traefik reads from files
/// (Swarm). Empty where Traefik is its own ACME client (standalone).
fn generate_traefik_dynamic_config(ingress: &IngressResolvedSpec, path: &Path, cert_domains: &[String]) -> Result<()> {
    let mut file = File::create(path)?;
    // Clients use https whenever TLS is on, upstream or here; only the latter
    // puts the routers on `websecure`.
    let public_https = ingress.tls.is_some();
    let has_tls = ingress.terminates_tls();
    let use_le = ingress.tls.as_ref().map(|t| t.letsencrypt.is_some()).unwrap_or(false);

    writeln!(file, "http:")?;

    let mut middlewares_written = false;
    // The same domain can appear in more than one rule (e.g. declared under
    // multiple host groups), so every generated name must include the rule
    // index `i` in addition to the service index `j`; using only the domain and
    // `j` would emit duplicate YAML keys that Traefik rejects.
    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");
        for (j, svc) in rule.services.iter().enumerate() {
            if svc.strip_prefix && svc.prefix != "/" {
                if !middlewares_written {
                    writeln!(file, "  middlewares:")?;
                    middlewares_written = true;
                }
                writeln!(file, "    strip-{}-{}-{}:", router_name_base, i, j)?;
                writeln!(file, "      stripPrefix:")?;
                writeln!(file, "        prefixes:")?;
                writeln!(file, "          - \"{}\"", svc.prefix)?;
            }
        }
    }

    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");
        for (j, svc) in rule.services.iter().enumerate() {
            // Traefik has no plain "reject oversized" switch: `buffering` is the
            // only middleware that caps a request body, and it spools the body
            // before forwarding it.
            if let Some(limit) = svc.body_limit {
                if !middlewares_written {
                    writeln!(file, "  middlewares:")?;
                    middlewares_written = true;
                }
                writeln!(file, "    limit-{}-{}-{}:", router_name_base, i, j)?;
                writeln!(file, "      buffering:")?;
                writeln!(file, "        maxRequestBodyBytes: {}", limit)?;
            }
        }
    }

    for redirect in &ingress.redirects {
        if !middlewares_written {
            writeln!(file, "  middlewares:")?;
            middlewares_written = true;
        }
        // `redirectRegex` rather than `redirectScheme`, because the destination
        // is a different host, and the path and query have to survive the bounce.
        writeln!(file, "    {}:", traefik_redirect_name(&redirect.from_domain))?;
        writeln!(file, "      redirectRegex:")?;
        writeln!(file, "        regex: \"^https?://[^/]+/(.*)\"")?;
        writeln!(
            file,
            "        replacement: \"{}/${{1}}\"",
            redirect.target_url(public_https)
        )?;
        writeln!(file, "        permanent: {}", redirect.permanent)?;
    }

    // The controller, under its prefix on `controller.host`: the prefix is
    // stripped, so the controller sees `/health`, `/api/v1/...`.
    let controller_domains: &[String] = ingress.controller.as_ref().map(|c| c.domains.as_slice()).unwrap_or(&[]);
    if !controller_domains.is_empty() {
        if !middlewares_written {
            writeln!(file, "  middlewares:")?;
        }
        writeln!(file, "    {}:", TRAEFIK_CONTROLLER_STRIP)?;
        writeln!(file, "      stripPrefix:")?;
        writeln!(file, "        prefixes:")?;
        writeln!(
            file,
            "          - {}",
            serde_json::to_string(crate::controller::PREFIX)?
        )?;
    }

    writeln!(file, "  routers:")?;
    for (i, domain) in controller_domains.iter().enumerate() {
        writeln!(file, "    simpled-controller-{}:", i)?;
        // With the slash, so `/simpled` does not also take `/simpledshop`.
        let rule = format!("Host(`{}`) && PathPrefix(`{}/`)", domain, crate::controller::PREFIX);
        writeln!(file, "      rule: {}", serde_json::to_string(&rule)?)?;
        writeln!(file, "      service: {}", TRAEFIK_CONTROLLER_SERVICE)?;
        writeln!(file, "      middlewares:")?;
        writeln!(file, "        - {}", TRAEFIK_CONTROLLER_STRIP)?;
        writeln!(file, "      entryPoints:")?;
        if has_tls {
            writeln!(file, "        - websecure")?;
            write_router_tls(&mut file, use_le && cert_domains.is_empty())?;
        } else {
            writeln!(file, "        - web")?;
        }
    }
    if ingress.tls_upstream() {
        // No Host matcher, so it answers whatever Host the health check sends.
        writeln!(file, "    {}:", TRAEFIK_PING_ROUTER)?;
        writeln!(file, "      rule: \"Path(`/ping`)\"")?;
        writeln!(file, "      entryPoints:")?;
        writeln!(file, "        - web")?;
        writeln!(file, "      service: ping@internal")?;
        writeln!(file, "      priority: 2147483647")?;
    }
    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");

        for (j, svc) in rule.services.iter().enumerate() {
            let router_name = format!("{}-{}-{}", router_name_base, i, j);
            writeln!(file, "    {}:", router_name)?;

            let path_rule = if svc.prefix == "/" {
                String::new()
            } else {
                format!(" && PathPrefix(`{}`)", svc.prefix)
            };

            writeln!(file, "      rule: \"Host(`{}`){}\"", rule.domain_name, path_rule)?;
            writeln!(file, "      service: service-{}-{}-{}", router_name_base, i, j)?;

            if has_tls {
                writeln!(file, "      entryPoints:")?;
                writeln!(file, "        - websecure")?;
                write_router_tls(&mut file, use_le && cert_domains.is_empty())?;
            } else {
                writeln!(file, "      entryPoints:")?;
                writeln!(file, "        - web")?;
            }

            let strips = svc.strip_prefix && svc.prefix != "/";
            if strips || svc.body_limit.is_some() {
                writeln!(file, "      middlewares:")?;
                if strips {
                    writeln!(file, "        - strip-{}-{}-{}", router_name_base, i, j)?;
                }
                if svc.body_limit.is_some() {
                    writeln!(file, "        - limit-{}-{}-{}", router_name_base, i, j)?;
                }
            }
        }
    }

    for redirect in &ingress.redirects {
        let name = traefik_redirect_name(&redirect.from_domain);
        writeln!(file, "    {}:", name)?;
        writeln!(file, "      rule: \"Host(`{}`)\"", redirect.from_domain)?;
        // A router needs a service even when the middleware answers first; the
        // dummy below is never reached.
        writeln!(file, "      service: {}", TRAEFIK_REDIRECT_SERVICE)?;
        if has_tls {
            writeln!(file, "      entryPoints:")?;
            writeln!(file, "        - websecure")?;
            write_router_tls(&mut file, use_le && cert_domains.is_empty())?;
        } else {
            writeln!(file, "      entryPoints:")?;
            writeln!(file, "        - web")?;
        }
        writeln!(file, "      middlewares:")?;
        writeln!(file, "        - {}", name)?;
    }

    // HTTP-01 challenges, on every domain over plain HTTP: the controller is the
    // ACME client, and every gateway task routes them to it.
    if !cert_domains.is_empty() {
        writeln!(file, "    simpled-acme-challenge:")?;
        writeln!(
            file,
            "      rule: \"PathPrefix(`{}`)\"",
            crate::controller::acme::CHALLENGE_PREFIX
        )?;
        writeln!(file, "      service: {}", TRAEFIK_CONTROLLER_SERVICE)?;
        writeln!(file, "      entryPoints:")?;
        writeln!(file, "        - web")?;
    }

    writeln!(file, "  services:")?;
    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");
        for (j, svc) in rule.services.iter().enumerate() {
            writeln!(file, "    service-{}-{}-{}:", router_name_base, i, j)?;
            writeln!(file, "      loadBalancer:")?;
            writeln!(file, "        servers:")?;
            writeln!(
                file,
                "          - url: \"http://{}_{}:{}/\"",
                svc.deployment_name, svc.service_name, svc.port
            )?;
        }
    }

    // The controller answers for every deployment, by the request's Host, so one
    // service serves every domain's routes.
    if !cert_domains.is_empty() || !controller_domains.is_empty() {
        writeln!(file, "    {}:", TRAEFIK_CONTROLLER_SERVICE)?;
        writeln!(file, "      loadBalancer:")?;
        writeln!(file, "        servers:")?;
        writeln!(
            file,
            "          - url: \"http://{}:{}/\"",
            crate::controller::HOST,
            crate::controller::PORT
        )?;
    }

    if !ingress.redirects.is_empty() {
        // Every router must name a service. The redirect middleware answers
        // before the request is forwarded, so this address is never dialled.
        writeln!(file, "    {}:", TRAEFIK_REDIRECT_SERVICE)?;
        writeln!(file, "      loadBalancer:")?;
        writeln!(file, "        servers:")?;
        writeln!(file, "          - url: \"http://127.0.0.1:1/\"")?;
    }

    // Each domain's certificate and key, in one PEM: a Swarm secret per domain.
    // Traefik picks the certificate by SNI.
    if !cert_domains.is_empty() {
        writeln!(file, "tls:")?;
        writeln!(file, "  certificates:")?;
        for domain in cert_domains {
            let path = crate::gateway_tls::cert_path(domain);
            writeln!(file, "    - certFile: {}", path)?;
            writeln!(file, "      keyFile: {}", path)?;
        }
    }

    Ok(())
}

/// A router's `tls`: Traefik's ACME resolver, or the certificates from the
/// `tls` section.
fn write_router_tls(file: &mut File, resolver: bool) -> Result<()> {
    if resolver {
        writeln!(file, "      tls:")?;
        writeln!(file, "        certResolver: {}", TRAEFIK_RESOLVER)?;
    } else {
        writeln!(file, "      tls: {{}}")?;
    }
    Ok(())
}

/// Traefik router/middleware names may not contain dots.
fn traefik_redirect_name(from_domain: &str) -> String {
    format!("redirect-{}", from_domain.replace(['.', ':'], "-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::{
        ConfigResolvedFile, ConfigResolvedSpec, DeploymentResolvedSpec, IngressRule, IngressTlsResolvedSpec,
        IngressToServiceRule, RedirectRule, SecretResolvedSpec, SecretResolvedValue, ServiceResolvedSpec,
    };
    use crate::spec::{
        AwsSecretRef, DeploymentEnvType, Healthcheck, HealthcheckTest, ResourceLimits, ResourcesSpec,
        ServiceConfigOption, ServiceSecret, ServiceType, ServiceVolume,
    };

    fn service(name: &str, service_type: ServiceType, depends_on: &[&str]) -> ServiceResolvedSpec {
        ServiceResolvedSpec {
            service_type,
            is_app_service: true,
            full_name: name.to_string(),
            image: format!("registry.example.com/{}:1.0.0", name),
            environment_variables: vec![],
            undockerized_environment_variables: vec![],
            configs: vec![],
            secrets: vec![],
            ports: vec![],
            expose: vec![],
            volumes: vec![],
            command: None,
            entrypoint: None,
            healthcheck: None,
            depends_on: (!depends_on.is_empty()).then(|| depends_on.iter().map(|d| d.to_string()).collect()),
            resources: ResourcesSpec {
                replicas: 1,
                requests: ResourceLimits::default(),
                limits: ResourceLimits::default(),
                update: None,
                placement: None,
            },
            working_dir: None,
            excluded: false,
        }
    }

    #[test]
    fn swarm_stack_scales_a_service_to_its_replicas() {
        let mut api = service("api", ServiceType::Public, &[]);
        api.resources.replicas = 3;
        let spec = spec(vec![api, service("worker", ServiceType::Internal, &[])]);
        let (dir, _script) = generate_swarm_to_temp(&spec);

        let stack = fs::read_to_string(dir.path().join("prod").join("docker-compose.yaml")).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
        assert_eq!(
            parsed["services"]["api"]["deploy"]["replicas"].as_u64(),
            Some(3),
            "{}",
            stack
        );
        assert_eq!(
            parsed["services"]["worker"]["deploy"]["replicas"].as_u64(),
            Some(1),
            "{}",
            stack
        );
        // Swarm names containers itself; the key is only meaningful to compose.
        assert!(parsed["services"]["api"]["container_name"].is_null(), "{}", stack);
    }

    fn with_secrets(mut service: ServiceResolvedSpec, secrets: &[(&str, SecretMount)]) -> ServiceResolvedSpec {
        service.secrets = secrets
            .iter()
            .map(|(name, mount)| ServiceSecret {
                name: name.to_string(),
                mount: mount.clone(),
            })
            .collect();
        service
    }

    fn aws_secret(name: &str, secret_id: &str, jq: Option<&str>) -> SecretResolvedSpec {
        SecretResolvedSpec {
            name: name.to_string(),
            value: SecretResolvedValue::Deferred(AwsSecretRef {
                secret_id: secret_id.to_string(),
                jq: jq.map(str::to_string),
            }),
        }
    }

    fn healthy(mut service: ServiceResolvedSpec) -> ServiceResolvedSpec {
        service.healthcheck = Some(Healthcheck {
            test: HealthcheckTest::Shell("pg_isready".to_string()),
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: false,
        });
        service
    }

    fn spec(services: Vec<ServiceResolvedSpec>) -> EnvironmentResolvedSpec {
        EnvironmentResolvedSpec {
            ports: Default::default(),
            env_type: DeploymentEnvType::Docker(DockerSpecificSpec {
                swarm_mode: true,
                multi_node: true,
                target_platform: Default::default(),
            }),
            ingress: IngressResolvedSpec {
                deployments: vec![],
                version_routes: vec![],
                version_document: None,
                controller: None,
                trusted_proxies: Vec::new(),
                access_log: None,
                global: false,
                placement: None,
                name: "gateway".to_string(),
                tls: None,
                domains: vec![],
                rules: vec![],
                redirects: vec![],
            },
            current_deployment: DeploymentResolvedSpec {
                name: "prod".to_string(),
                application_name: "shop".to_string(),
                application_version: "1.0.0".to_string(),
                configs: vec![],
                secrets: vec![],
                services,
                volumes: vec![],
                volume_bindings: Default::default(),
                host_environment: vec![],
            },
        }
    }

    fn ingress_with_redirect(tls: Option<IngressTlsResolvedSpec>) -> IngressResolvedSpec {
        IngressResolvedSpec {
            deployments: vec![],
            version_routes: vec![],
            version_document: None,
            controller: None,
            trusted_proxies: Vec::new(),
            access_log: None,
            global: false,
            placement: None,
            name: "gateway".to_string(),
            tls,
            domains: vec!["www.somesite.com".to_string(), "somesite.com".to_string()],
            rules: vec![IngressRule {
                domain_name: "www.somesite.com".to_string(),
                services: vec![IngressToServiceRule {
                    service_name: "api".to_string(),
                    deployment_name: "prod".to_string(),
                    port: 80,
                    prefix: "/".to_string(),
                    strip_prefix: false,
                    body_limit: None,
                }],
            }],
            redirects: vec![RedirectRule {
                from_domain: "somesite.com".to_string(),
                to: "www.somesite.com".to_string(),
                permanent: true,
            }],
        }
    }

    fn ingress_with_limits(limits: &[(&str, &str, Option<u64>)]) -> IngressResolvedSpec {
        IngressResolvedSpec {
            deployments: vec![],
            version_routes: vec![],
            version_document: None,
            controller: None,
            trusted_proxies: Vec::new(),
            access_log: None,
            global: false,
            placement: None,
            name: "gateway".to_string(),
            tls: None,
            domains: vec!["somesite.com".to_string()],
            rules: vec![IngressRule {
                domain_name: "somesite.com".to_string(),
                services: limits
                    .iter()
                    .map(|(name, prefix, limit)| IngressToServiceRule {
                        service_name: name.to_string(),
                        deployment_name: "prod".to_string(),
                        port: 80,
                        prefix: prefix.to_string(),
                        strip_prefix: false,
                        body_limit: *limit,
                    })
                    .collect(),
            }],
            redirects: vec![],
        }
    }

    #[test]
    fn traefik_caps_the_body_with_a_buffering_middleware() {
        let ingress = ingress_with_limits(&[("upload", "/upload", Some(10 * 1024 * 1024))]);
        let conf = write_config(&ingress);

        assert!(conf.contains("    limit-somesite-com-0-0:"), "{}", conf);
        assert!(conf.contains("      buffering:"), "{}", conf);
        assert!(conf.contains("        maxRequestBodyBytes: 10485760"), "{}", conf);
        assert!(conf.contains("        - limit-somesite-com-0-0"), "{}", conf);
    }

    #[test]
    fn traefik_keeps_both_middlewares_when_a_route_strips_and_caps() {
        let mut ingress = ingress_with_limits(&[("upload", "/upload", Some(2048))]);
        ingress.rules[0].services[0].strip_prefix = true;
        let conf = write_config(&ingress);

        assert!(conf.contains("        - strip-somesite-com-0-0"), "{}", conf);
        assert!(conf.contains("        - limit-somesite-com-0-0"), "{}", conf);
    }

    fn letsencrypt_tls() -> IngressTlsResolvedSpec {
        IngressTlsResolvedSpec {
            secret: None,
            letsencrypt: Some(LetsEncryptResolvedSpec {
                server: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
                email: "ops@somesite.com".to_string(),
            }),
            upstream: false,
        }
    }

    fn write_config(ingress: &IngressResolvedSpec) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conf");
        generate_traefik_dynamic_config(ingress, &path, &[]).unwrap();
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn traefik_redirects_with_a_middleware_on_a_router_of_its_own() {
        let conf = write_config(&ingress_with_redirect(Some(letsencrypt_tls())));

        assert!(conf.contains("    redirect-somesite-com:"), "{}", conf);
        assert!(
            conf.contains("        replacement: \"https://www.somesite.com/${1}\""),
            "{}",
            conf
        );
        assert!(conf.contains("        permanent: true"), "{}", conf);
        assert!(conf.contains("      rule: \"Host(`somesite.com`)\""), "{}", conf);
        // Redirect-only routers still have to name a backend.
        assert!(
            conf.contains(&format!("      service: {}", TRAEFIK_REDIRECT_SERVICE)),
            "{}",
            conf
        );
        assert!(conf.contains(&format!("    {}:", TRAEFIK_REDIRECT_SERVICE)), "{}", conf);
    }

    #[test]
    fn a_temporary_redirect_answers_302() {
        let mut ingress = ingress_with_redirect(None);
        ingress.redirects[0].permanent = false;

        assert!(write_config(&ingress).contains("        permanent: false"));
    }

    fn generate_swarm_to_temp(spec: &EnvironmentResolvedSpec) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let docker_spec = DockerSpecificSpec {
            swarm_mode: true,
            multi_node: true,
            target_platform: Default::default(),
        };
        generate(spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();
        (dir, script)
    }

    #[test]
    fn swarm_creates_missing_absolute_bind_sources_but_leaves_existing_paths_alone() {
        let mut db = service("db", ServiceType::Internal, &[]);
        db.volumes = vec![
            ServiceVolume {
                name: ServiceVolumeType::Path("/var/lib/app_db".to_string()),
                mount_path: "/var/lib/mysql".to_string(),
            },
            ServiceVolume {
                name: ServiceVolumeType::Path("./data".to_string()),
                mount_path: "/data".to_string(),
            },
        ];
        let mut ui = service("ui", ServiceType::Internal, &[]);
        // A socket, not a directory: `mkdir -p` would fail on it.
        ui.volumes = vec![ServiceVolume {
            name: ServiceVolumeType::Path("/var/run/docker.sock".to_string()),
            mount_path: "/var/run/docker.sock".to_string(),
        }];
        let (_dir, script) = generate_swarm_to_temp(&spec(vec![db, ui]));

        assert!(
            script.contains("[ -e \"/var/lib/app_db\" ] || mkdir -p \"/var/lib/app_db\""),
            "{}",
            script
        );
        assert!(
            script.contains("[ -e \"/var/run/docker.sock\" ] || mkdir -p \"/var/run/docker.sock\""),
            "{}",
            script
        );
        // Absolute paths are never created unconditionally.
        assert!(
            !script.contains(
                "
mkdir -p \"/var/"
            ),
            "{}",
            script
        );
        // Relative mounts keep the plain form, under the deployment directory.
        assert!(script.contains("mkdir -p \"prod/data\""), "{}", script);

        // All of it has to happen before the stack is deployed.
        let ensure = script.find("[ -e \"/var/lib/app_db\" ]").unwrap();
        let deploy = script.find("docker stack deploy").unwrap();
        assert!(ensure < deploy, "{}", script);
    }

    #[test]
    fn swarm_without_jobs_deploys_the_stack_in_one_step() {
        let spec = spec(vec![service("api", ServiceType::Public, &[])]);
        let (dir, script) = generate_swarm_to_temp(&spec);

        assert!(script.contains(
            "docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth --prune --detach=false\n"
        ));
        assert!(!script.contains("Phase 1/3"));
        assert!(!script.contains("run_job"));
        assert!(!dir.path().join("prod").join(DEPS_COMPOSE_FILE).exists());
    }

    /// Pruned before the rollout, the image of the release that is still
    /// running survives; pruned after it, that rollback image would be gone.
    #[test]
    fn swarm_prunes_images_before_anything_is_deployed() {
        let spec = spec(vec![service("api", ServiceType::Public, &[])]);
        let (_dir, script) = generate_swarm_to_temp(&spec);

        let prune = script.find("docker image prune -af").unwrap();
        let first_deploy = script.find("docker stack deploy").unwrap();
        assert!(prune < first_deploy, "{}", script);
        assert_eq!(script.matches("docker image prune").count(), 1, "{}", script);
    }

    #[test]
    fn swarm_job_runs_between_two_partial_rollouts() {
        let spec = spec(vec![
            service("api", ServiceType::Public, &["migrate"]),
            healthy(service("primary-db", ServiceType::Internal, &[])),
            service("migrate", ServiceType::Job, &["primary-db"]),
        ]);
        let (dir, script) = generate_swarm_to_temp(&spec);

        // Phase 1 brings up only the job's dependency and waits for convergence.
        let phase1 = script
            .find("docker stack deploy -c prod/docker-compose.deps.yaml prod --with-registry-auth --detach=false")
            .unwrap();
        // The deps-only subset must not prune, or phase 3 would have to bring
        // back every service it removed.
        assert!(!script.contains("docker-compose.deps.yaml prod --with-registry-auth --prune"));
        let job = script.find("./simpled step run-job prod/plan.json 'migrate'").unwrap();
        let phase3 = script
            .find("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth --prune --detach=false\n")
            .unwrap();
        assert!(
            phase1 < job && job < phase3,
            "phases must be ordered deps -> job -> stack"
        );

        // The dependency stack file holds the database only.
        let deps = fs::read_to_string(dir.path().join("prod").join(DEPS_COMPOSE_FILE)).unwrap();
        assert!(deps.contains("primary-db:"));
        assert!(!deps.contains("api:"));
        assert!(!deps.contains("migrate:"));

        // Jobs never end up in the stack file: `docker stack deploy` cannot run
        // them to completion.
        let stack = fs::read_to_string(dir.path().join("prod").join("docker-compose.yaml")).unwrap();
        assert!(stack.contains("primary-db:") && stack.contains("api:"));
        assert!(!stack.contains("migrate:"));

        // The job still gets its generated environment file, through the plan.
        assert!(dir.path().join("prod").join("migrate").join(".env").exists());
        let plan = Plan::read(&dir.path().join("prod").join("plan.json")).unwrap();
        let job = plan.job("migrate").unwrap();
        assert_eq!(job.service, "prod_migrate");
        assert_eq!(job.timeout_seconds, JOB_TIMEOUT_SECONDS);
        assert!(
            job.args
                .windows(2)
                .any(|w| w == ["--env-file", "${DEPLOY_DIR}/prod/migrate/.env"]),
            "{:?}",
            job.args
        );
        assert!(
            job.args.windows(2).any(|w| w == ["--network", DOCKER_NETWORK]),
            "{:?}",
            job.args
        );
    }

    #[test]
    fn job_without_depends_on_waits_for_the_whole_stack() {
        let spec = spec(vec![
            service("api", ServiceType::Public, &[]),
            service("migrate", ServiceType::Job, &[]),
        ]);
        let (dir, script) = generate_swarm_to_temp(&spec);

        // Nothing declared, so everything long-running is a prerequisite: phase 1
        // deploys the full stack file and phase 3 has nothing left to do.
        let phase1 = script
            .find("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth --prune --detach=false")
            .unwrap();
        let job = script.find("./simpled step run-job prod/plan.json 'migrate'").unwrap();
        assert!(phase1 < job);
        assert!(script.contains("All services were started in phase 1"));
        assert!(!dir.path().join("prod").join(DEPS_COMPOSE_FILE).exists());
    }

    #[test]
    fn job_with_external_dependencies_runs_before_anything_starts() {
        // An explicit empty list: the database is managed outside the stack, so
        // the migration runs first and the whole stack follows it.
        let mut migrate = service("migrate", ServiceType::Job, &[]);
        migrate.depends_on = Some(vec![]);
        let spec = spec(vec![service("api", ServiceType::Public, &[]), migrate]);
        let (dir, script) = generate_swarm_to_temp(&spec);

        let nothing_first = script
            .find("No job dependencies declared, nothing to start first.")
            .unwrap();
        let job = script.find("./simpled step run-job prod/plan.json 'migrate'").unwrap();
        let stack = script
            .find("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth --prune --detach=false")
            .unwrap();
        assert!(nothing_first < job && job < stack, "{}", script);
        assert_eq!(script.matches("docker stack deploy -c prod/").count(), 1, "{}", script);
        assert!(!dir.path().join("prod").join(DEPS_COMPOSE_FILE).exists());
    }

    #[test]
    fn jobs_run_in_dependency_order() {
        let spec = spec(vec![
            healthy(service("primary-db", ServiceType::Internal, &[])),
            service("seed", ServiceType::Job, &["migrate", "primary-db"]),
            service("migrate", ServiceType::Job, &["primary-db"]),
        ]);
        let (_dir, script) = generate_swarm_to_temp(&spec);

        let migrate = script.find("./simpled step run-job prod/plan.json 'migrate'").unwrap();
        let seed = script.find("./simpled step run-job prod/plan.json 'seed'").unwrap();
        assert!(migrate < seed, "a job that depends on another job runs after it");
    }

    #[test]
    fn standalone_runs_jobs_in_the_foreground_after_their_dependencies() {
        let mut spec = spec(vec![
            service("api", ServiceType::Public, &[]),
            healthy(service("primary-db", ServiceType::Internal, &[])),
            service("migrate", ServiceType::Job, &["primary-db"]),
        ]);
        let docker_spec = DockerSpecificSpec {
            swarm_mode: false,
            multi_node: false,
            target_platform: Default::default(),
        };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();

        let db = script.find("docker run -d --name primary-db").unwrap();
        let wait = script.find("wait_healthy primary-db").unwrap();
        // A job blocks the script, so it is started without -d.
        let job = script.find("docker run --name migrate").unwrap();
        let api = script.find("docker run -d --name api").unwrap();
        assert!(db < wait && wait < job && job < api);
    }

    /// The resolver prefixes config and secret names with the application name,
    /// so the generator must not prefix them again: the run commands mount the
    /// prefixed name, and a second prefix wrote the files where nothing read them
    /// (docker then created a directory at the missing mount source).
    #[test]
    fn standalone_writes_configs_and_secrets_where_the_run_commands_mount_them() {
        let mut service = service("api", ServiceType::Public, &[]);
        service.configs = vec![ServiceConfigOption {
            config_name: "shop-data".to_string(),
            mount_path: "/data".to_string(),
        }];
        service.secrets = vec![ServiceSecret {
            name: "shop-tls_key".to_string(),
            mount: SecretMount::FilePath("/run/secrets/tls.key".to_string()),
        }];

        let mut spec = spec(vec![service]);
        spec.current_deployment.configs = vec![ConfigResolvedSpec {
            name: "shop-data".to_string(),
            files: vec![ConfigResolvedFile {
                name: "settings.json".to_string(),
                content: b"{}".to_vec(),
            }],
        }];
        spec.current_deployment.secrets = vec![SecretResolvedSpec {
            name: "shop-tls_key".to_string(),
            value: SecretResolvedValue::Literal("pem".to_string()),
        }];
        let docker_spec = DockerSpecificSpec {
            swarm_mode: false,
            multi_node: false,
            target_platform: Default::default(),
        };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();

        assert!(script.contains("-v $(pwd)/configs/shop-data:/data"));
        assert!(script.contains("-v $(pwd)/secrets/shop-tls_key:/run/secrets/tls.key"));
        assert_eq!(
            fs::read_to_string(dir.path().join("configs").join("shop-data").join("settings.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("secrets").join("shop-tls_key")).unwrap(),
            "pem"
        );
        assert!(!dir.path().join("configs").join("shop-shop-data").exists());
    }

    #[test]
    fn swarm_defers_aws_secrets_to_the_deploy_target() {
        let mut spec = spec(vec![with_secrets(
            service("api", ServiceType::Public, &[]),
            &[
                ("shop-db_password", SecretMount::EnvVariable("DB_PASSWORD".to_string())),
                (
                    "shop-tls_key",
                    SecretMount::FilePath("/run/secrets/tls.key".to_string()),
                ),
            ],
        )]);
        spec.current_deployment.secrets = vec![
            aws_secret("shop-db_password", "prod/shop/db", Some(".password")),
            aws_secret("shop-tls_key", "prod/shop/tls", None),
        ];

        let (dir, script) = generate_swarm_to_temp(&spec);

        // The deploy script sources the fetch script before it deploys anything,
        // so the exported values reach the stack file's ${...} interpolation.
        let source = script.find(". ./fetch-secrets.sh").unwrap();
        let deploy = script.find("docker stack deploy").unwrap();
        assert!(source < deploy);

        let fetch = fs::read_to_string(dir.path().join("fetch-secrets.sh")).unwrap();
        assert!(fetch.contains(
            "SIMPLED_SECRET_SHOP_DB_PASSWORD=\"$(simpled_read_secret \
             'shop-db_password' 'prod/shop/db' '.password')\" || exit 1"
        ));
        assert!(fetch.contains("export SIMPLED_SECRET_SHOP_DB_PASSWORD"));
        // Only the secret that asked for a filter pulls jq into the requirements.
        assert!(fetch.contains("command -v jq"));
        // The file a file secret's Swarm object is created from is written by the
        // fetch script, and only then synced.
        assert!(fetch.contains("printf '%s' \"$SIMPLED_SECRET_SHOP_TLS_KEY\" > 'prod/secrets/shop-tls_key'"));
        assert!(!dir.path().join("prod").join("secrets").join("shop-tls_key").exists());
        let sync = script
            .find("objects=\"$(./simpled step sync-objects prod/plan.json)\"")
            .expect("the deployment's objects are synced");
        assert!(source < sync && sync < deploy, "{}", script);
        let plan = Plan::read(&dir.path().join("prod").join("plan.json")).unwrap();
        assert_eq!(plan.owner_label, "simpled.deployment=prod");
        assert_eq!(plan.objects[0].base, "prod_shop-tls_key");
        assert_eq!(plan.objects[0].file, "prod/secrets/shop-tls_key");
        assert_eq!(plan.objects[0].var, "SIMPLED_OBJECT_1");
        // A secret with an `aws` source is only fetched on the target, so
        // nothing about its value goes into the plan.
        let text = fs::read_to_string(dir.path().join("prod").join("plan.json")).unwrap();
        assert!(!text.contains("prod/shop/tls"), "{}", text);

        // The stack file references the env secret through its variable, and the
        // file secret as an external Swarm secret.
        let stack = fs::read_to_string(dir.path().join("prod").join("docker-compose.yaml")).unwrap();
        assert!(stack.contains("DB_PASSWORD: ${SIMPLED_SECRET_SHOP_DB_PASSWORD}"));
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stack).unwrap();
        let mount = &parsed["services"]["api"]["secrets"][0];
        assert_eq!(mount["source"].as_str(), Some("prod_shop-tls_key"), "{}", stack);
        assert_eq!(mount["target"].as_str(), Some("/run/secrets/tls.key"), "{}", stack);
        assert_eq!(mount["mode"].as_u64(), Some(0o444), "{}", stack);
        assert_eq!(parsed["secrets"]["prod_shop-tls_key"]["external"].as_bool(), Some(true));
        assert_eq!(
            parsed["secrets"]["prod_shop-tls_key"]["name"].as_str(),
            Some("${SIMPLED_OBJECT_1}"),
            "{}",
            stack
        );
        assert!(parsed["services"]["api"]["volumes"].is_null(), "{}", stack);

        // Pruned only at the end, after every stack deploy succeeded.
        let prune = script
            .find("./simpled step prune-objects prod/plan.json")
            .expect("old generations are pruned");
        assert!(script.rfind("docker stack deploy").unwrap() < prune, "{}", script);
    }

    #[test]
    fn swarm_job_expands_deferred_secrets_in_the_shell() {
        let mut spec = spec(vec![with_secrets(
            service("migrate", ServiceType::Job, &[]),
            &[("shop-db_password", SecretMount::EnvVariable("DB_PASSWORD".to_string()))],
        )]);
        spec.current_deployment.secrets = vec![aws_secret("shop-db_password", "prod/shop/db", None)];

        let (dir, _script) = generate_swarm_to_temp(&spec);

        // The job's plan carries the same template the stack file does, which
        // `step run-job` expands from the variable fetch-secrets.sh exported.
        let plan = Plan::read(&dir.path().join("prod").join("plan.json")).unwrap();
        let args = &plan.job("migrate").unwrap().args;
        assert!(
            args.windows(2)
                .any(|w| w == ["-e", "DB_PASSWORD=${SIMPLED_SECRET_SHOP_DB_PASSWORD}"]),
            "{:?}",
            args
        );
    }

    #[test]
    fn a_literal_dollar_reaches_a_job_unchanged() {
        let mut spec = spec(vec![with_secrets(
            service("migrate", ServiceType::Job, &[]),
            &[("shop-db_password", SecretMount::EnvVariable("DB_PASSWORD".to_string()))],
        )]);
        spec.current_deployment.secrets = vec![SecretResolvedSpec {
            name: "shop-db_password".to_string(),
            value: SecretResolvedValue::Literal("pa$word".to_string()),
        }];
        let (dir, _script) = generate_swarm_to_temp(&spec);

        let plan = Plan::read(&dir.path().join("prod").join("plan.json")).unwrap();
        let args = &plan.job("migrate").unwrap().args;
        let value = args.iter().find(|a| a.starts_with("DB_PASSWORD=")).unwrap();
        // Escaped for compose, and expanded back by the step the way compose does.
        let expanded = crate::deploy_plan::interpolate(value, |_| None).unwrap();
        assert_eq!(expanded, "DB_PASSWORD=pa$word");
    }

    #[test]
    fn standalone_writes_deferred_secrets_where_the_run_commands_read_them() {
        let mut spec = spec(vec![with_secrets(
            service("api", ServiceType::Public, &[]),
            &[
                ("shop-db_password", SecretMount::EnvVariable("DB_PASSWORD".to_string())),
                ("shop-api_key", SecretMount::EnvVariable("API_KEY".to_string())),
            ],
        )]);
        spec.current_deployment.secrets = vec![
            aws_secret("shop-db_password", "prod/shop/db", None),
            SecretResolvedSpec {
                name: "shop-api_key".to_string(),
                value: SecretResolvedValue::Literal("k3y".to_string()),
            },
        ];
        let docker_spec = DockerSpecificSpec {
            swarm_mode: false,
            multi_node: false,
            target_platform: Default::default(),
        };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();

        assert!(script.contains(". ./fetch-secrets.sh"));
        // Both kinds are read back the same way; only where the file comes from
        // differs, so the names have to line up with what the resolver produced.
        assert!(script.contains("-e DB_PASSWORD=\"$(cat ./secrets/shop-db_password)\""));
        let fetch = fs::read_to_string(dir.path().join("fetch-secrets.sh")).unwrap();
        assert!(fetch.contains("printf '%s' \"$SIMPLED_SECRET_SHOP_DB_PASSWORD\" > 'secrets/shop-db_password'"));
        assert!(!dir.path().join("secrets").join("shop-db_password").exists());

        // A secret that was resolvable here is still written out as before.
        let literal = fs::read_to_string(dir.path().join("secrets").join("shop-api_key")).unwrap();
        assert_eq!(literal, "k3y");
    }

    #[test]
    fn standalone_quotes_the_image_entrypoint_and_arguments() {
        let mut api = service("api", ServiceType::Public, &[]);
        api.entrypoint = Some(crate::spec::ServiceCommand::Exec(vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
        ]));
        api.command = Some(crate::spec::ServiceCommand::Exec(vec![
            "echo it's alive && sleep 1".to_string()
        ]));
        let mut spec = spec(vec![api]);
        let docker_spec = DockerSpecificSpec {
            swarm_mode: false,
            multi_node: false,
            target_platform: Default::default(),
        };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();

        // entrypoint ++ command, each token one shell word.
        assert!(
            script.contains(
                "--entrypoint '/bin/sh' 'registry.example.com/api:1.0.0' '-c' 'echo it'\\''s alive && sleep 1'"
            ),
            "{}",
            script
        );
    }

    /// Traefik targets `<deployment>_<service>`, the DNS name a Swarm service gets.
    /// A standalone container is only reachable by its own name unless it is
    /// given that alias, so without it every route returned a bad gateway.
    #[test]
    fn standalone_containers_carry_the_name_traefik_targets() {
        let mut spec = spec(vec![service("api", ServiceType::Public, &[])]);
        spec.ingress = ingress_with_limits(&[("api", "/", None)]);
        let docker_spec = DockerSpecificSpec {
            swarm_mode: false,
            multi_node: false,
            target_platform: Default::default(),
        };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();
        let traefik = fs::read_to_string(dir.path().join("traefik").join("dynamic_conf.yml")).unwrap();

        assert!(traefik.contains("url: \"http://prod_api:80/\""), "{}", traefik);
        assert!(
            script.contains("docker run -d --name api --network common_network --network-alias prod_api"),
            "{}",
            script
        );
    }

    fn static_config(has_tls: bool, trusted_proxies: &[&str]) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traefik.yml");
        let tls = letsencrypt_tls();
        let letsencrypt = if has_tls { tls.letsencrypt.as_ref() } else { None };
        let proxies: Vec<String> = trusted_proxies.iter().map(|p| p.to_string()).collect();
        let mut file = File::create(&path).unwrap();
        write_traefik_static_config(&mut file, has_tls, letsencrypt, &proxies, false, None).unwrap();
        drop(file);
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn traefik_static_config_is_unchanged_without_trusted_proxies() {
        assert_eq!(
            static_config(true, &[]),
            "entryPoints:\n  web:\n    address: \":80\"\n    http:\n      redirections:\n        entryPoint:\n          to: websecure\n          scheme: https\n  websecure:\n    address: \":443\"\nproviders:\n  file:\n    filename: \"/etc/traefik/dynamic_conf.yml\"\n    watch: true\ncertificatesResolvers:\n  myresolver:\n    acme:\n      email: \"ops@somesite.com\"\n      storage: \"/letsencrypt/acme.json\"\n      httpChallenge:\n        entryPoint: web\n"
        );
        assert_eq!(
            static_config(false, &[]),
            "entryPoints:\n  web:\n    address: \":80\"\nproviders:\n  file:\n    filename: \"/etc/traefik/dynamic_conf.yml\"\n    watch: true\n"
        );
    }

    #[test]
    fn upstream_tls_traefik_listens_on_web_only_and_answers_ping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("traefik.yml");
        let mut file = File::create(&path).unwrap();
        write_traefik_static_config(&mut file, false, None, &["10.0.0.0/16".to_string()], true, None).unwrap();
        drop(file);
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "entryPoints:\n  web:\n    address: \":80\"\n    forwardedHeaders:\n      trustedIPs:\n        - \"10.0.0.0/16\"\nproviders:\n  file:\n    filename: \"/etc/traefik/dynamic_conf.yml\"\n    watch: true\nping:\n  entryPoint: web\n  manualRouting: true\n"
        );
    }

    #[test]
    fn the_access_log_goes_to_stdout_with_only_x_forwarded_for_kept() {
        let conf = |format: AccessLogFormat| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("traefik.yml");
            let mut file = File::create(&path).unwrap();
            write_traefik_static_config(&mut file, false, None, &[], false, Some(format)).unwrap();
            drop(file);
            serde_yaml::from_str::<serde_yaml::Value>(&fs::read_to_string(path).unwrap()).unwrap()
        };
        let json = conf(AccessLogFormat::Json);
        let log = &json["accessLog"];
        assert_eq!(log["format"], "json");
        assert!(log.get("filePath").is_none(), "stdout, not a file");
        assert_eq!(log["fields"]["headers"]["defaultMode"], "drop");
        assert_eq!(log["fields"]["headers"]["names"]["X-Forwarded-For"], "keep");
        let common = conf(AccessLogFormat::Common);
        assert_eq!(
            common["accessLog"],
            serde_yaml::from_str::<serde_yaml::Value>("format: common").unwrap()
        );
    }

    #[test]
    fn upstream_tls_routers_stay_on_web_and_redirect_to_https() {
        let mut ingress = ingress_with_redirect(Some(IngressTlsResolvedSpec {
            secret: None,
            letsencrypt: None,
            upstream: true,
        }));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dynamic_conf.yml");
        generate_traefik_dynamic_config(&ingress, &path, &[]).unwrap();
        let conf = fs::read_to_string(&path).unwrap();
        assert!(!conf.contains("websecure") && !conf.contains("tls:"), "{}", conf);
        assert!(
            conf.contains("replacement: \"https://www.somesite.com/${1}\""),
            "{}",
            conf
        );
        assert!(
            conf.contains(
                "  routers:\n    simpled-ping:\n      rule: \"Path(`/ping`)\"\n      entryPoints:\n        - web\n      service: ping@internal\n      priority: 2147483647\n"
            ),
            "{}",
            conf
        );

        // Without TLS at all the redirect stays on http, and there is no ping.
        ingress.tls = None;
        generate_traefik_dynamic_config(&ingress, &path, &[]).unwrap();
        let conf = fs::read_to_string(&path).unwrap();
        assert!(!conf.contains("simpled-ping"), "{}", conf);
        assert!(
            conf.contains("replacement: \"http://www.somesite.com/${1}\""),
            "{}",
            conf
        );
    }

    #[test]
    fn traefik_trusts_forwarded_headers_from_the_proxies_on_both_entrypoints() {
        let conf = static_config(true, &["10.0.0.0/8", "2600:9000::/28"]);
        let parsed: serde_yaml::Value = serde_yaml::from_str(&conf).unwrap();
        for entrypoint in ["web", "websecure"] {
            let trusted = &parsed["entryPoints"][entrypoint]["forwardedHeaders"]["trustedIPs"];
            assert_eq!(
                trusted
                    .as_sequence()
                    .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
                Some(vec!["10.0.0.0/8", "2600:9000::/28"]),
                "{}",
                conf
            );
        }
        assert_eq!(
            parsed["entryPoints"]["web"]["http"]["redirections"]["entryPoint"]["to"].as_str(),
            Some("websecure"),
            "{}",
            conf
        );
    }
}
