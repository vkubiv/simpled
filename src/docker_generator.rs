use std::collections::HashMap;
use crate::resolved_spec::{EnvironmentResolvedSpec, IngressResolvedSpec, LetsEncryptResolvedSpec, SHELL_VAR_PREFIX};
use crate::secret_fetch::{self, sh_quote, FetchScript};
use crate::spec::{DockerIngressType, DockerSpecificSpec, SecretMount, ServiceVolumeType};
use anyhow::{anyhow, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use crate::docker_compose::{prepare_service, DockerCompose, DockerComposeNetwork, DockerService, ServiceNetwork};

const DOCKER_NETWORK: &str = "common_network";
const NGINX_IMAGE: &str = "nginx:alpine";
const TRAEFIK_IMAGE: &str = "traefik:v2.10";
const TRAEFIK_RESOLVER: &str = "myresolver";
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

/// `run_job` helper embedded in the generated Swarm deploy script.
///
/// Swarm's run-to-completion primitive (`--mode replicated-job`) exists only in
/// the CLI/API — compose's `deploy.mode` does not accept it, so a job placed in a
/// stack file is deployed as an ordinary replicated service that never converges.
/// Creating the job as a standalone service instead gives a real pass/fail signal.
const RUN_JOB_FUNCTION: &str = r#"run_job() {
  job_name="$1"
  shift
  echo "Running job ${job_name}..."

  # A service left over from an earlier deploy (or from an older simpled that put
  # jobs in the stack file) would make `docker service create` fail on the name.
  docker service rm "${job_name}" >/dev/null 2>&1 || true

  create_status=0
  docker service create \
    --name "${job_name}" \
    --mode replicated-job \
    --restart-condition none \
    --detach=false \
    --with-registry-auth \
    "$@" || create_status=$?

  # The task is polled for a terminal state rather than trusting the exit code of
  # `docker service create` alone, which reports on the rollout, not on the
  # process the job ran.
  waited=0
  state=""
  while [ "${waited}" -lt "${JOB_TIMEOUT}" ]; do
    state="$(docker service ps "${job_name}" --format '{{.CurrentState}}' 2>/dev/null | head -n 1)"
    case "${state}" in
      Complete*|Failed*|Rejected*|Orphaned*|Shutdown*) break ;;
    esac
    sleep 2
    waited=$((waited + 2))
  done

  case "${state}" in
    Complete*)
      docker service logs --tail 50 "${job_name}" || true
      docker service rm "${job_name}" >/dev/null 2>&1 || true
      echo "Job ${job_name} completed."
      ;;
    *)
      echo "Job ${job_name} did not complete (state: ${state:-unknown}, create exit code: ${create_status})." >&2
      docker service logs --tail 200 "${job_name}" >&2 || true
      docker service rm "${job_name}" >/dev/null 2>&1 || true
      exit 1
      ;;
  esac
}
"#;

pub fn generate(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }
    
    if docker_spec.swarm_mode {
        generate_swarm(resolved_spec, docker_spec, output_dir)
    } else {
        generate_standalone(resolved_spec, docker_spec, output_dir)
    }
}

fn generate_standalone(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
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
        let path = secrets_dir.join(&secret.name);
        match secret.deferred() {
            Some(reference) => {
                fetch_script.fetch(secret, reference);
                fetch_script.write_to_file(secret, &format!("secrets/{}", secret.name));
            }
            None => fs::write(&path, secret.literal().unwrap_or_default())?,
        }
    }
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
    let mut deploy_sh = File::create(output_dir.join("deploy.sh"))?;
    
    #[cfg(unix)]
    {
        let mut perms = deploy_sh.metadata()?.permissions();
        perms.set_mode(0o755);
        deploy_sh.set_permissions(perms)?;
    }

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;

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
        writeln!(deploy_sh, "HEALTH_TIMEOUT=\"${{HEALTH_TIMEOUT:-{}}}\"", HEALTH_TIMEOUT_SECONDS)?;
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

         for env in &service.environment_variables {
             writeln!(env_file, "{}={}", env.name, env.value)?;
         }

         // A job runs in the foreground: `set -e` then aborts the deploy when the
         // job exits non-zero, before any service that depends on its result
         // (e.g. a migrated schema) is started.
         let detach = if is_job { "" } else { "-d " };
         write!(deploy_sh, "docker run {}--name {} --network {}", detach, service.full_name, network_name)?;

         for port in &service.ports {
             write!(deploy_sh, " -p {}:{}", port.external, port.internal)?;
         }

         // `expose` declares internal-only ports (reachable by other containers
         // on the network but not published to the host).
         for port in &service.expose {
             write!(deploy_sh, " --expose {}", port)?;
         }

         write!(deploy_sh, " --env-file $(pwd)/envs/{}", env_file_name)?;
         
         for secret in &service.secrets {
              if let SecretMount::EnvVariable(var_name) = &secret.mount {
                   write!(deploy_sh, " -e {}=$(cat ./secrets/{})", var_name, secret.name)?;
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
                     write!(deploy_sh, " --health-cmd '{}'", cmd.replace('\'', "'\\''"))?;
                 }
                 if let Some(v) = &hc.interval { write!(deploy_sh, " --health-interval {}", v)?; }
                 if let Some(v) = &hc.timeout { write!(deploy_sh, " --health-timeout {}", v)?; }
                 if let Some(v) = hc.retries { write!(deploy_sh, " --health-retries {}", v)?; }
                 if let Some(v) = &hc.start_period { write!(deploy_sh, " --health-start-period {}", v)?; }
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
                 write!(deploy_sh, " --entrypoint {}", args.remove(0))?;
                 trailing_args.extend(args);
             }
         }
         if let Some(command) = &service.command {
             trailing_args.extend(command.to_args());
         }

         write!(deploy_sh, " {}", service.image)?;
         for arg in trailing_args {
             write!(deploy_sh, " {}", arg)?;
         }
         writeln!(deploy_sh)?;
    }
    
    // Ingress Container
    if !resolved_spec.ingress.rules.is_empty() {
        match docker_spec.ingress_type {
            DockerIngressType::Nginx => generate_nginx_standalone(resolved_spec, output_dir, &mut deploy_sh, network_name)?,

            DockerIngressType::Traefik => {
                generate_traefik_standalone(resolved_spec, output_dir, &mut deploy_sh, network_name)?;
            }
        }

    }
    
    Ok(())
}

fn generate_swarm(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    let deployment = &resolved_spec.current_deployment;

    // Application folder
    let app_dir = output_dir.join(&deployment.name);
    fs::create_dir_all(&app_dir)?;

    for volume in &deployment.volumes {
        let volume_sub_dir = app_dir.clone().join("volumes").join(&volume);
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

    for service in &deployment.services {
        let mut docker_service = prepare_service(service, resolved_spec, &app_dir)?;

        let mut networks = HashMap::new();
        networks.insert("default".to_string(), ServiceNetwork {
            aliases: vec![service.full_name.clone()],
        });

        docker_service.networks = networks;

        // File-mounted deferred secrets: the bind mount is in the stack file, but
        // the file it points at is only created once the script has run. The path
        // mirrors what `prepare_service` uses, relative to the deploy directory
        // rather than to the stack file.
        for secret_option in &service.secrets {
            let SecretMount::FilePath(mount_path) = &secret_option.mount else { continue };
            let Some(secret) = deployment.secrets.iter()
                .find(|s| s.name == secret_option.name && s.deferred().is_some()) else { continue };
            let rel_path = mount_path.trim_start_matches('/');
            fetch_script.write_to_file(secret, &format!(
                "{}/{}/{}", deployment.name, service.full_name, rel_path));
        }

        prepared.insert(service.full_name.clone(), docker_service);
    }

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
        let mut networks = HashMap::new();
        networks.insert("default".to_string(), DockerComposeNetwork {
            external: true,
            name: network_name.clone(),
        });
        networks
    };

    let stack_services: HashMap<String, DockerService> = long_running.iter()
        .filter_map(|s| prepared.get(&s.full_name).map(|d| (s.full_name.clone(), d.clone())))
        .collect();

    let compose = DockerCompose {
        services: stack_services,
        networks: compose_network(),
    };

    let compose_path = &app_dir.join("docker-compose.yaml");
    let yaml = serde_yaml::to_string(&compose)?;
    fs::write(&compose_path, yaml)?;

    // Phase-1 stack file: only the services the jobs need. Written when it is a
    // real subset — when the jobs need everything, phase 1 deploys the full stack
    // file instead and phase 3 has nothing left to do.
    let deps_only = !jobs.is_empty()
        && !prerequisites.is_empty()
        && prerequisites.len() < long_running.len();

    if deps_only {
        let dep_services: HashMap<String, DockerService> = prerequisites.iter()
            .filter_map(|s| prepared.get(&s.full_name).map(|d| (s.full_name.clone(), d.clone())))
            .collect();

        let deps_compose = DockerCompose {
            services: dep_services,
            networks: compose_network(),
        };

        let deps_path = app_dir.join(DEPS_COMPOSE_FILE);
        fs::write(&deps_path, serde_yaml::to_string(&deps_compose)?)?;
    }



    // 4. Ingress Stack
    let ingress_dir = output_dir.join("ingress");
    fs::create_dir_all(&ingress_dir)?;
    
    match docker_spec.ingress_type {
        DockerIngressType::Nginx => generate_nginx_swarm(resolved_spec, &ingress_dir, network_name.clone())?,
        DockerIngressType::Traefik => generate_traefik_swarm(resolved_spec, &ingress_dir, network_name.clone())?,
    }

    // 5. Deploy Script
    let mut deploy_sh = File::create(output_dir.join("deploy.sh"))?;
    
    #[cfg(unix)]
    {
        let mut perms = deploy_sh.metadata()?.permissions();
        perms.set_mode(0o755);
        deploy_sh.set_permissions(perms)?;
    }

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;
    write_fetch_secrets_call(&mut deploy_sh, &fetch_script)?;
    if !jobs.is_empty() {
        writeln!(deploy_sh, "# Absolute paths are needed for the bind mounts of jobs, which are created")?;
        writeln!(deploy_sh, "# with `docker service create` instead of being part of the stack file.")?;
        writeln!(deploy_sh, "DEPLOY_DIR=\"$(pwd)\"")?;
        writeln!(deploy_sh, "# Seconds to wait for a single job to finish before giving up.")?;
        writeln!(deploy_sh, "JOB_TIMEOUT=\"${{JOB_TIMEOUT:-{}}}\"", JOB_TIMEOUT_SECONDS)?;
        writeln!(deploy_sh)?;
        write_run_job_function(&mut deploy_sh)?;
    }
    writeln!(deploy_sh, "docker network create --driver overlay --attachable {} || true", network_name)?;

    // Bind-mounted volume directories are not created automatically on the node
    // during the first deployment, which makes `docker stack deploy` fail. Collect
    // every host directory the stack binds and ensure it exists (mkdir -p is a
    // no-op when the directory already exists). Paths are relative to the compose
    // file location, i.e. the deployment directory.
    let mut volume_dirs: Vec<String> = Vec::new();

    // Named volumes are bound as `./volumes/<name>` inside the deployment dir.
    for volume in &deployment.volumes {
        volume_dirs.push(format!("{}/volumes/{}", deployment.name, volume));
    }

    // Service-level relative path mounts (e.g. `./data:/var/lib/...`).
    for service in &deployment.services {
        for volume in &service.volumes {
            if let ServiceVolumeType::Path(from_path) = &volume.name {
                if let Some(rel) = from_path.strip_prefix("./") {
                    volume_dirs.push(format!("{}/{}", deployment.name, rel));
                }
            }
        }
    }

    volume_dirs.sort();
    volume_dirs.dedup();

    if !volume_dirs.is_empty() {
        writeln!(deploy_sh, "echo 'Ensuring volume directories exist...'")?;
        for dir in &volume_dirs {
            writeln!(deploy_sh, "mkdir -p \"{}\"", dir)?;
        }
    }

    writeln!(deploy_sh, "docker stack deploy -c ingress/docker-compose.yaml ingress --detach=false")?;

    if jobs.is_empty() {
        writeln!(deploy_sh, "docker stack deploy -c {}/docker-compose.yaml {} --with-registry-auth", deployment.name, deployment.name)?;
    } else {
        // Jobs are run between two partial rollouts of the same stack. `docker
        // stack deploy` only removes services missing from the file when it is
        // given --prune, so deploying a subset first and the full file afterwards
        // is an additive, idempotent rollout: phase 3 reports the phase-1
        // services as up to date and leaves them running.
        writeln!(deploy_sh)?;
        writeln!(deploy_sh, "echo '== Phase 1/3: starting services the jobs depend on =='")?;
        if prerequisites.is_empty() {
            writeln!(deploy_sh, "echo 'No job dependencies declared, nothing to start first.'")?;
        } else {
            let phase1_file = if deps_only { DEPS_COMPOSE_FILE } else { "docker-compose.yaml" };
            // --detach=false blocks until every service in the file has converged,
            // i.e. its tasks are running and (when a healthcheck is declared)
            // healthy. That is what makes it safe to run the jobs next.
            writeln!(deploy_sh, "docker stack deploy -c {}/{} {} --with-registry-auth --detach=false",
                deployment.name, phase1_file, deployment.name)?;
        }

        writeln!(deploy_sh)?;
        writeln!(deploy_sh, "echo '== Phase 2/3: running jobs =='")?;
        for job in &jobs {
            let docker_service = prepared.get(&job.full_name)
                .ok_or_else(|| anyhow!("Job {} was not prepared", job.full_name))?;
            write_job_invocation(&mut deploy_sh, &deployment.name, &job.full_name, docker_service, &network_name)?;
        }

        writeln!(deploy_sh)?;
        writeln!(deploy_sh, "echo '== Phase 3/3: deploying the rest of the stack =='")?;
        if long_running.is_empty() {
            writeln!(deploy_sh, "echo 'This deployment has no long-running services.'")?;
        } else if deps_only || prerequisites.is_empty() {
            writeln!(deploy_sh, "docker stack deploy -c {}/docker-compose.yaml {} --with-registry-auth", deployment.name, deployment.name)?;
        } else {
            // Phase 1 already deployed every long-running service.
            writeln!(deploy_sh, "echo 'All services were started in phase 1, nothing left to deploy.'")?;
        }
    }

    // After a successful deploy, reclaim disk space by removing images that are no
    // longer used by any container/service (e.g. the previous versions replaced by
    // this rollout). `set -e` above guarantees this only runs when the deploy
    // succeeded.
    //
    // `docker stack deploy` returns before the rollout has converged, so the new
    // tasks may still be pulling/starting their images. Wait 3 minutes to give the
    // rollout time to settle before pruning, otherwise we could remove an image a
    // task still depends on.
    writeln!(deploy_sh, "echo 'Waiting for rollout to settle before pruning...'")?;
    writeln!(deploy_sh, "sleep 180")?;
    writeln!(deploy_sh, "echo 'Pruning unused images...'")?;
    writeln!(deploy_sh, "docker image prune -af")?;

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
    writeln!(deploy_sh, "# Read the secrets with an `aws` source from AWS Secrets Manager. Sourced, so")?;
    writeln!(deploy_sh, "# the values it exports are visible to the commands below.")?;
    writeln!(deploy_sh, ". ./{}", secret_fetch::SCRIPT_NAME)?;
    writeln!(deploy_sh)?;
    Ok(())
}

fn write_run_job_function(deploy_sh: &mut File) -> Result<()> {
    write!(deploy_sh, "{}", RUN_JOB_FUNCTION)?;
    writeln!(deploy_sh)?;
    Ok(())
}

/// Emit the `run_job` call for one job, translating the compose representation of
/// the service into `docker service create` flags. Everything a job needs is
/// covered: env file, inline environment (including env-variable secrets), bind
/// mounts (configs, file secrets, volumes), entrypoint and command. Ports and
/// healthchecks are not translated — neither is meaningful for a task that is
/// expected to exit.
fn write_job_invocation(
    deploy_sh: &mut File,
    deployment_name: &str,
    job_name: &str,
    service: &DockerService,
    network_name: &str,
) -> Result<()> {
    // Named like a stack service so it is recognizable in `docker service ls`
    // while it runs; the job is removed again once it completes.
    let service_name = format!("{}_{}", deployment_name, job_name);

    writeln!(deploy_sh, "run_job {} \\", sh_quote(&service_name))?;
    writeln!(deploy_sh, "  --network {} \\", sh_quote(network_name))?;

    for env_file in &service.env_file {
        writeln!(deploy_sh, "  --env-file \"{}\" \\", node_path(deployment_name, env_file))?;
    }

    let mut env_names: Vec<&String> = service.environment.keys().collect();
    env_names.sort();
    for name in env_names {
        let value = &service.environment[name];
        // A deferred secret is carried as a `${SIMPLED_SECRET_*}` placeholder that
        // the stack file has `docker stack deploy` interpolate. A job bypasses the
        // stack file, so the placeholder has to be left unquoted for the shell to
        // expand it here instead. Everything else is quoted verbatim.
        if is_secret_placeholder(value) {
            writeln!(deploy_sh, "  -e \"{}={}\" \\", name, value)?;
        } else {
            writeln!(deploy_sh, "  -e {} \\", sh_quote(&format!("{}={}", name, value)))?;
        }
    }

    for volume in &service.volumes {
        // Compose volume entries are `<source>:<target>`; the source is relative
        // to the stack file, i.e. to the deployment directory.
        let Some((source, target)) = volume.split_once(':') else {
            return Err(anyhow!("Job {} has an invalid volume entry '{}'", job_name, volume));
        };
        writeln!(deploy_sh, "  --mount \"type=bind,source={},target={}\" \\",
            node_path(deployment_name, source), target)?;
    }

    // `docker service create --entrypoint` overrides only the executable, so any
    // remaining entrypoint tokens are prepended to the container args, the same
    // split `docker run` needs. Requires a Docker CLI that supports the flag
    // (25.0+); jobs that do not override the entrypoint work on any version.
    let mut trailing_args: Vec<String> = Vec::new();
    if let Some(entrypoint) = &service.entrypoint {
        let mut args = entrypoint.to_args();
        if !args.is_empty() {
            writeln!(deploy_sh, "  --entrypoint {} \\", sh_quote(&args.remove(0)))?;
            trailing_args.extend(args);
        }
    }
    if let Some(command) = &service.command {
        trailing_args.extend(command.to_args());
    }

    write!(deploy_sh, "  {}", sh_quote(&service.image))?;
    for arg in trailing_args {
        write!(deploy_sh, " {}", sh_quote(&arg))?;
    }
    writeln!(deploy_sh)?;

    Ok(())
}

/// Whether an env value is the `${SIMPLED_SECRET_*}` reference `prepare_service`
/// writes for a secret that is only fetched on the deploy target.
fn is_secret_placeholder(value: &str) -> bool {
    value.starts_with(&format!("${{{}", SHELL_VAR_PREFIX)) && value.ends_with('}')
}

/// Absolute path on the deployment node for a path written relative to the stack
/// file. Already-absolute paths (a user-declared host mount) are left alone.
fn node_path(deployment_name: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("$DEPLOY_DIR/{}/{}", deployment_name, path.trim_start_matches("./"))
    }
}

fn generate_nginx_standalone(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path, deploy_sh: &mut File, network_name: String) -> Result<()> {
    if !resolved_spec.ingress.rules.is_empty() {
        let nginx_dir = output_dir.join("nginx");
        fs::create_dir_all(&nginx_dir)?;
        generate_nginx_config(&resolved_spec.ingress, &nginx_dir.join("default.conf"))?;
    }

    writeln!(deploy_sh, "echo 'Starting Nginx ingress...'")?;
    writeln!(deploy_sh, "docker rm -f nginx-ingress || true")?;
    write!(deploy_sh, "docker run -d --name nginx-ingress --network {}", network_name)?;
    write!(deploy_sh, " -p 80:80")?;

    let has_tls = resolved_spec.ingress.tls.is_some();
    if has_tls {
        write!(deploy_sh, " -p 443:443")?;
    }

    write!(deploy_sh, " -v $(pwd)/nginx/default.conf:/etc/nginx/conf.d/default.conf")?;

    if has_tls {
        fs::create_dir_all(output_dir.join("certs"))?;
        write!(deploy_sh, " -v $(pwd)/certs:/etc/nginx/certs")?;
    }

    if let Some(tls) = &resolved_spec.ingress.tls {
        if tls.letsencrypt.is_some() {
            write!(deploy_sh, " -v $(pwd)/letsencrypt:/var/www/letsencrypt")?;
            fs::create_dir_all(output_dir.join("letsencrypt"))?;
        }
    }

    write!(deploy_sh, " -e DEPLOY_DATE=$(date +%s)")?;
    writeln!(deploy_sh, " {}", NGINX_IMAGE)?;

    if let Some(tls) = &resolved_spec.ingress.tls {
        if let Some(le) = &tls.letsencrypt {
            let mut certbot_sh = File::create(output_dir.join("certbot.sh"))?;
            writeln!(certbot_sh, "docker run -it --rm --name certbot \\")?;
            writeln!(certbot_sh, "  -v $(pwd)/letsencrypt:/var/www/letsencrypt \\")?;
            writeln!(certbot_sh, "  -v $(pwd)/certs:/etc/nginx/certs \\")?;
            writeln!(certbot_sh, "  certbot/certbot certonly --webroot --webroot-path=/var/www/letsencrypt \\")?;
            writeln!(certbot_sh, "  --email {} --agree-tos --no-eff-email \\", le.email)?;
            for domain in &resolved_spec.ingress.domains {
                writeln!(certbot_sh, "   -d {} \\", domain)?;
            }
            writeln!(certbot_sh, "   && docker restart nginx-ingress")?;
        }
    }
    Ok(())
}

fn generate_nginx_swarm(resolved_spec: &EnvironmentResolvedSpec, ingress_dir: &Path, network_name: String) -> Result<()> {
    if resolved_spec.ingress.rules.is_empty() {
        return Ok(());
    }

    let nginx_conf_dir = ingress_dir.join("nginx");
    fs::create_dir_all(&nginx_conf_dir)?;
    generate_nginx_config(&resolved_spec.ingress, &nginx_conf_dir.join("default.conf"))?;
    
    let mut stack = File::create(ingress_dir.join("docker-compose.yaml"))?;
    writeln!(stack, "version: '3.8'")?;
    writeln!(stack, "services:")?;
    writeln!(stack, "  nginx:")?;
    writeln!(stack, "    image: {}", NGINX_IMAGE)?;
    writeln!(stack, "    ports:")?;
    writeln!(stack, "      - \"80:80\"")?;
    if resolved_spec.ingress.tls.is_some() {
        writeln!(stack, "      - \"443:443\"")?;
    }
    writeln!(stack, "    volumes:")?;
    writeln!(stack, "      - ./nginx/default.conf:/etc/nginx/conf.d/default.conf")?;
    
    if resolved_spec.ingress.tls.is_some() {
         // We assume certs are placed in output_dir/certs -> so from ingress/docker-compose.yaml, it is ../certs
         // Wait, the structure is output_dir/ingress/docker-compose.yaml
         // So ../certs is output_dir/certs
         writeln!(stack, "      - ../certs:/etc/nginx/certs")?;
         
         if let Some(tls) = &resolved_spec.ingress.tls {
            if tls.letsencrypt.is_some() {
                 writeln!(stack, "      - ../letsencrypt:/var/www/letsencrypt")?;
            }
         }
    }
    
    write_swarm_compose_network(&mut stack, &network_name)?;

    Ok(())
}

fn generate_nginx_config(ingress: &IngressResolvedSpec, path: &Path) -> Result<()> {
    let mut file = File::create(path)?;

    let has_tls = ingress.tls.is_some();

    // The same domain can be declared under multiple host groups, producing
    // several rules with the same domain_name. nginx treats repeated server
    // blocks with the same server_name as a conflict and silently ignores all
    // but the first, dropping those routes, so merge every rule's services under
    // a single server block per domain. `domains` preserves first-seen order.
    let mut domains: Vec<&String> = Vec::new();
    let mut services_by_domain: HashMap<&String, Vec<&crate::resolved_spec::IngressToServiceRule>> =
        HashMap::new();
    for rule in &ingress.rules {
        if !services_by_domain.contains_key(&rule.domain_name) {
            domains.push(&rule.domain_name);
        }
        services_by_domain
            .entry(&rule.domain_name)
            .or_default()
            .extend(rule.services.iter());
    }

    for domain in domains {
        let services = &services_by_domain[domain];

        writeln!(file, "server {{")?;
        writeln!(file, "    listen 80;")?;
        writeln!(file, "    server_name {};", domain)?;

        if let Some(tls) = &ingress.tls {
            if tls.letsencrypt.is_some() {
                writeln!(file, "    location /.well-known/acme-challenge/ {{")?;
                writeln!(file, "        root /var/www/letsencrypt;")?;
                writeln!(file, "    }}")?;
            }
        }

        if has_tls {
            writeln!(file, "    location / {{")?;
            writeln!(file, "        return 301 https://$host$request_uri;")?;
            writeln!(file, "    }}")?;
            writeln!(file, "}}")?;

            writeln!(file, "server {{")?;
            writeln!(file, "    listen 443 ssl;")?;
            writeln!(file, "    server_name {};", domain)?;
            writeln!(file, "    ssl_certificate /etc/nginx/certs/live/{}/fullchain.pem;", domain)?;
            writeln!(file, "    ssl_certificate_key /etc/nginx/certs/live/{}/privkey.pem;", domain)?;

            generate_locations(&mut file, services)?;

            writeln!(file, "}}")?;

        } else {
            generate_locations(&mut file, services)?;
            writeln!(file, "}}")?;
        }
    }

    Ok(())
}

fn generate_locations(
    file: &mut File,
    services: &[&crate::resolved_spec::IngressToServiceRule],
) -> Result<()> {
    for svc in services {
        let prefix = &svc.prefix;
        let location_path = if prefix.ends_with('/') {
            prefix.clone()
        } else {
            format!("{}/", prefix)
        };
        
        writeln!(file, "    location {} {{", location_path)?;
        
        if svc.strip_prefix {
            writeln!(file, "        proxy_pass http://{}:{}/;", svc.service_name, svc.port)?;
        } else {
            writeln!(file, "        proxy_pass http://{}:{};", svc.service_name, svc.port)?;
        }
        
        writeln!(file, "        proxy_set_header Host $host;")?;
        writeln!(file, "        proxy_set_header X-Real-IP $remote_addr;")?;
        writeln!(file, "        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;")?;
        writeln!(file, "        proxy_set_header X-Forwarded-Proto $scheme;")?;
        writeln!(file, "    }}")?;
    }
    Ok(())
}

fn generate_traefik_standalone(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path, deploy_sh: &mut File, network_name: String) -> Result<()> {
    let traefik_dir = output_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    let has_tls = resolved_spec.ingress.tls.is_some();
    let letsencrypt = resolved_spec.ingress.tls.as_ref().and_then(|t| t.letsencrypt.as_ref());

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    write_traefik_static_config(&mut static_conf, has_tls, letsencrypt)?;

    generate_traefik_dynamic_config(&resolved_spec.ingress, &traefik_dir.join("dynamic_conf.yml"))?;

    writeln!(deploy_sh, "echo 'Starting Traefik ingress...'")?;
    writeln!(deploy_sh, "docker rm -f traefik-ingress || true")?;
    
    write!(deploy_sh, "docker run -d --name traefik-ingress --network {}", network_name)?;
    write!(deploy_sh, " -p 80:80")?;
    if has_tls {
        write!(deploy_sh, " -p 443:443")?;
    }

    write!(deploy_sh, " -v $(pwd)/traefik/traefik.yml:/etc/traefik/traefik.yml")?;
    write!(deploy_sh, " -v $(pwd)/traefik/dynamic_conf.yml:/etc/traefik/dynamic_conf.yml")?;

    if letsencrypt.is_some() {
        let le_dir = output_dir.join("letsencrypt");
        fs::create_dir_all(&le_dir)?;
        write!(deploy_sh, " -v $(pwd)/letsencrypt:/letsencrypt")?;
    }

    write!(deploy_sh, " -e DEPLOY_DATE=$(date +%s)")?;
    writeln!(deploy_sh, " {}", TRAEFIK_IMAGE)?;

    Ok(())
}

fn generate_traefik_swarm(resolved_spec: &EnvironmentResolvedSpec, ingress_dir: &Path, network_name: String) -> Result<()> {
    let traefik_dir = ingress_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    // Reuse generate logic for config, but write to new dir
    let has_tls = resolved_spec.ingress.tls.is_some();
    let letsencrypt = resolved_spec.ingress.tls.as_ref().and_then(|t| t.letsencrypt.as_ref());

    if letsencrypt.is_none() {
        return Err(anyhow!("Currently swarm ingress only supports Let's Encrypt, specify a letsencrypt block in ingress.tls"));
    }

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    write_traefik_static_config(&mut static_conf, has_tls, letsencrypt)?;

    generate_traefik_dynamic_config(&resolved_spec.ingress, &traefik_dir.join("dynamic_conf.yml"))?;

    let mut stack = File::create(ingress_dir.join("docker-compose.yaml"))?;
    writeln!(stack, "version: '3.8'")?;
    writeln!(stack, "services:")?;
    writeln!(stack, "  traefik:")?;
    writeln!(stack, "    image: {}", TRAEFIK_IMAGE)?;
    writeln!(stack, "    ports:")?;
    writeln!(stack, "      - \"80:80\"")?;
    if has_tls {
        writeln!(stack, "      - \"443:443\"")?;
    }
    writeln!(stack, "    volumes:")?;
    writeln!(stack, "      - ./traefik/traefik.yml:/etc/traefik/traefik.yml")?;
    writeln!(stack, "      - ./traefik/dynamic_conf.yml:/etc/traefik/dynamic_conf.yml")?;
    
    // Mount letsencrypt if needed. Using ../letsencrypt as in nginx
    if letsencrypt.is_some() {
        writeln!(stack, "      - ../letsencrypt:/letsencrypt")?;
        // Make sure dir exists
        fs::create_dir_all(ingress_dir.parent().unwrap().join("letsencrypt"))?;
    }

    write_swarm_compose_network(&mut stack, &network_name)?;

    Ok(())
}

fn write_traefik_static_config(file: &mut File, has_tls: bool, letsencrypt: Option<&LetsEncryptResolvedSpec>) -> Result<()> {
    writeln!(file, "entryPoints:")?;
    writeln!(file, "  web:")?;
    writeln!(file, "    address: \":80\"")?;
    if has_tls {
        writeln!(file, "    http:")?;
        writeln!(file, "      redirections:")?;
        writeln!(file, "        entryPoint:")?;
        writeln!(file, "          to: websecure")?;
        writeln!(file, "          scheme: https")?;
        writeln!(file, "  websecure:")?;
        writeln!(file, "    address: \":443\"")?;
    }
    writeln!(file, "providers:")?;
    writeln!(file, "  file:")?;
    writeln!(file, "    filename: \"/etc/traefik/dynamic_conf.yml\"")?;
    writeln!(file, "    watch: true")?;
    if let Some(le) = letsencrypt {
        writeln!(file, "certificatesResolvers:")?;
        writeln!(file, "  {}:", TRAEFIK_RESOLVER)?;
        writeln!(file, "    acme:")?;
        writeln!(file, "      email: \"{}\"", le.email)?;
        writeln!(file, "      storage: \"/letsencrypt/acme.json\"")?;
        writeln!(file, "      httpChallenge:")?;
        writeln!(file, "        entryPoint: web")?;
    }
    Ok(())
}

fn write_swarm_compose_network(stack: &mut File, network_name: &str) -> Result<()> {
    let deploy_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    writeln!(stack, "    environment:")?;
    writeln!(stack, "      - DEPLOY_DATE={}", deploy_date)?;
    writeln!(stack, "    networks:")?;
    writeln!(stack, "      default:")?;
    writeln!(stack, "networks:")?;
    writeln!(stack, "  default:")?;
    writeln!(stack, "    external: true")?;
    writeln!(stack, "    name: {}", network_name)?;
    Ok(())
}

fn generate_traefik_dynamic_config(ingress: &IngressResolvedSpec, path: &Path) -> Result<()> {
    let mut file = File::create(path)?;
    let has_tls = ingress.tls.is_some();
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
    
    writeln!(file, "  routers:")?;
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
                 writeln!(file, "      tls:")?;
                 if use_le {
                     writeln!(file, "        certResolver: {}", TRAEFIK_RESOLVER)?;
                 }
             } else {
                 writeln!(file, "      entryPoints:")?;
                 writeln!(file, "        - web")?;
             }
             
             if svc.strip_prefix && svc.prefix != "/" {
                  writeln!(file, "      middlewares:")?;
                  writeln!(file, "        - strip-{}-{}-{}", router_name_base, i, j)?;
             }
        }
    }
    
    writeln!(file, "  services:")?;
    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");
        for (j, svc) in rule.services.iter().enumerate() {
             writeln!(file, "    service-{}-{}-{}:", router_name_base, i, j)?;
             writeln!(file, "      loadBalancer:")?;
             writeln!(file, "        servers:")?;
             writeln!(file, "          - url: \"http://{}_{}:{}/\"", svc.deployment_name,  svc.service_name, svc.port)?;
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::{ConfigResolvedFile, ConfigResolvedSpec, DeploymentResolvedSpec, SecretResolvedSpec, SecretResolvedValue, ServiceResolvedSpec};
    use crate::spec::{AwsSecretRef, DeploymentEnvType, Healthcheck, HealthcheckTest, ResourceLimits, ResourcesSpec, ServiceConfigOption, ServiceSecret, ServiceType};

    fn service(name: &str, service_type: ServiceType, depends_on: &[&str]) -> ServiceResolvedSpec {
        ServiceResolvedSpec {
            service_type,
            is_app_service: true,
            full_name: name.to_string(),
            image: format!("registry.example.com/{}:1.0.0", name),
            service_host: "example.com".to_string(),
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
            depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
            working_dir: None,
        }
    }

    fn with_secrets(mut service: ServiceResolvedSpec, secrets: &[(&str, SecretMount)]) -> ServiceResolvedSpec {
        service.secrets = secrets.iter()
            .map(|(name, mount)| ServiceSecret { name: name.to_string(), mount: mount.clone() })
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
            interval: None, timeout: None, retries: None, start_period: None, disable: false,
        });
        service
    }

    fn spec(services: Vec<ServiceResolvedSpec>) -> EnvironmentResolvedSpec {
        EnvironmentResolvedSpec {
            env_type: DeploymentEnvType::Docker(DockerSpecificSpec {
                ingress_type: DockerIngressType::Nginx,
                swarm_mode: true,
            }),
            ingress: IngressResolvedSpec {
                name: "gateway".to_string(),
                tls: None,
                domains: vec![],
                rules: vec![],
            },
            current_deployment: DeploymentResolvedSpec {
                name: "prod".to_string(),
                application_name: "shop".to_string(),
                configs: vec![],
                secrets: vec![],
                defaults: ResourcesSpec {
                    replicas: 1,
                    requests: ResourceLimits { memory: "128Mi".to_string(), cpu: "100m".to_string() },
                    limits: ResourceLimits { memory: "256Mi".to_string(), cpu: "200m".to_string() },
                },
                services,
                volumes: vec![],
            },
        }
    }

    fn generate_swarm_to_temp(spec: &EnvironmentResolvedSpec) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let docker_spec = DockerSpecificSpec { ingress_type: DockerIngressType::Nginx, swarm_mode: true };
        generate(spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();
        (dir, script)
    }

    #[test]
    fn swarm_without_jobs_deploys_the_stack_in_one_step() {
        let spec = spec(vec![service("api", ServiceType::Public, &[])]);
        let (dir, script) = generate_swarm_to_temp(&spec);

        assert!(script.contains("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth\n"));
        assert!(!script.contains("Phase 1/3"));
        assert!(!script.contains("run_job"));
        assert!(!dir.path().join("prod").join(DEPS_COMPOSE_FILE).exists());
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
        let phase1 = script.find("docker stack deploy -c prod/docker-compose.deps.yaml prod --with-registry-auth --detach=false").unwrap();
        let job = script.find("run_job 'prod_migrate'").unwrap();
        let phase3 = script.find("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth\n").unwrap();
        assert!(phase1 < job && job < phase3, "phases must be ordered deps -> job -> stack");

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

        // The job still gets its generated environment file.
        assert!(dir.path().join("prod").join("migrate").join(".env").exists());
        assert!(script.contains("--env-file \"$DEPLOY_DIR/prod/migrate/.env\""));
        assert!(script.contains("--mode replicated-job"));
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
        let phase1 = script.find("docker stack deploy -c prod/docker-compose.yaml prod --with-registry-auth --detach=false").unwrap();
        let job = script.find("run_job 'prod_migrate'").unwrap();
        assert!(phase1 < job);
        assert!(script.contains("All services were started in phase 1"));
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

        let migrate = script.find("run_job 'prod_migrate'").unwrap();
        let seed = script.find("run_job 'prod_seed'").unwrap();
        assert!(migrate < seed, "a job that depends on another job runs after it");
    }

    #[test]
    fn standalone_runs_jobs_in_the_foreground_after_their_dependencies() {
        let mut spec = spec(vec![
            service("api", ServiceType::Public, &[]),
            healthy(service("primary-db", ServiceType::Internal, &[])),
            service("migrate", ServiceType::Job, &["primary-db"]),
        ]);
        let docker_spec = DockerSpecificSpec { ingress_type: DockerIngressType::Nginx, swarm_mode: false };
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
        let docker_spec = DockerSpecificSpec { ingress_type: DockerIngressType::Nginx, swarm_mode: false };
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
                ("shop-tls_key", SecretMount::FilePath("/run/secrets/tls.key".to_string())),
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
            "SIMPLED_SECRET_SHOP_DB_PASSWORD=\"$(aws secretsmanager get-secret-value \
             --secret-id 'prod/shop/db' --query SecretString --output text | jq -r '.password')\""
        ));
        assert!(fetch.contains("export SIMPLED_SECRET_SHOP_DB_PASSWORD"));
        // Only the secret that asked for a filter pulls jq into the requirements.
        assert!(fetch.contains("command -v jq"));
        assert!(fetch.contains("printf '%s' \"$SIMPLED_SECRET_SHOP_TLS_KEY\" > 'prod/api/run/secrets/tls.key'"));

        // The stack file references the secret instead of carrying its value, and
        // the file behind the bind mount is left for the fetch script to create.
        let stack = fs::read_to_string(dir.path().join("prod").join("docker-compose.yaml")).unwrap();
        assert!(stack.contains("DB_PASSWORD: ${SIMPLED_SECRET_SHOP_DB_PASSWORD}"));
        assert!(stack.contains("./api/run/secrets/tls.key:/run/secrets/tls.key"));
        assert!(!dir.path().join("prod").join("api").join("run/secrets/tls.key").exists());
    }

    #[test]
    fn swarm_job_expands_deferred_secrets_in_the_shell() {
        let mut spec = spec(vec![with_secrets(
            service("migrate", ServiceType::Job, &[]),
            &[("shop-db_password", SecretMount::EnvVariable("DB_PASSWORD".to_string()))],
        )]);
        spec.current_deployment.secrets = vec![aws_secret("shop-db_password", "prod/shop/db", None)];

        let (_dir, script) = generate_swarm_to_temp(&spec);

        // A job is created outside the stack file, so the placeholder has to be
        // left unquoted for the deploy script's own shell to expand it.
        assert!(script.contains("-e \"DB_PASSWORD=${SIMPLED_SECRET_SHOP_DB_PASSWORD}\""));
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
        let docker_spec = DockerSpecificSpec { ingress_type: DockerIngressType::Nginx, swarm_mode: false };
        spec.env_type = DeploymentEnvType::Docker(docker_spec.clone());

        let dir = tempfile::tempdir().unwrap();
        generate(&spec, &docker_spec, dir.path()).unwrap();
        let script = fs::read_to_string(dir.path().join("deploy.sh")).unwrap();

        assert!(script.contains(". ./fetch-secrets.sh"));
        // Both kinds are read back the same way; only where the file comes from
        // differs, so the names have to line up with what the resolver produced.
        assert!(script.contains("-e DB_PASSWORD=$(cat ./secrets/shop-db_password)"));
        let fetch = fs::read_to_string(dir.path().join("fetch-secrets.sh")).unwrap();
        assert!(fetch.contains("printf '%s' \"$SIMPLED_SECRET_SHOP_DB_PASSWORD\" > 'secrets/shop-db_password'"));
        assert!(!dir.path().join("secrets").join("shop-db_password").exists());

        // A secret that was resolvable here is still written out as before.
        let literal = fs::read_to_string(dir.path().join("secrets").join("shop-api_key")).unwrap();
        assert_eq!(literal, "k3y");
    }
}
