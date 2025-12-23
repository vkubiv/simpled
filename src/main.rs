use clap::{Parser, Subcommand};
use anyhow::{Context, Result, bail, anyhow};
use std::path::Path;

mod spec;
mod spec_yaml;
mod env_loader;
mod transform;
mod validator;
mod resolved_spec;
mod resolver;
mod k8s_generator;
mod docker_generator;
mod run_local;
mod local_ingress;
mod spec_loader;
mod app_bundle;

#[derive(Parser)]
#[command(name = "simpled")]
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
    /// Secrets management
    Secrets {
        #[command(subcommand)]
        command: SecretsCommands,
    },
    /// Prepare deployment (e.g. generate k8s manifests)
    PrepareDeployment {
        deployment_name: String,
        #[arg(long)]
        bundle: Option<String>,
        #[arg(long)]
        version: Option<String>,
    },

    /// Used for local development and tests
    Local {
        #[command(subcommand)]
        command: LocalCommands,
    }
}

#[derive(Subcommand)]
enum LocalCommands {
    Run {
        #[arg(short, long)]
        exclude: Option<Vec<String>>,

        #[arg(long)]
        path: Option<String>,
    }
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
        #[arg(long)]
        upload: Option<String>,
    },
}

#[derive(Subcommand)]
enum SecretsCommands {
    Set {
        env_name: String,
        path: Option<String>,
        #[arg(short = 'f', long)]
        file: Vec<String>,
    },
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    match &cli.command {
        Commands::AppBundle { command } => match command {
            AppBundleCommands::Verify => {
                verify_command()?;
            }
            AppBundleCommands::Version => {
                version_command()?;
            }
            AppBundleCommands::Create { registry, push_images, upload } => {
                app_bundle::create_app_bundle(registry, *push_images, upload)?;
            }
        },
        Commands::Secrets { command } => match command {
            SecretsCommands::Set { env_name, path, file } => {
                println!("Set secrets for {}, path={:?}, args={:?}", env_name, path, file);
            }
        },
        Commands::PrepareDeployment { deployment_name, bundle, version } => {
            prepare_deployment_command(deployment_name, bundle, version)?;
        },
        Commands::Local { command } => {
            local(&command)?;
        }
    }
    Ok(())
}

fn verify_command() -> Result<()> {
    let app_spec = spec_loader::load_app_spec(Path::new("."), None)?;
    println!("Successfully validated appspec: {} v{}", app_spec.name, app_spec.version);
    Ok(())
}

fn version_command() -> Result<()> {
    let app_spec = spec_loader::load_app_spec(Path::new("."), None)?;
    println!("{}", app_spec.version);
    Ok(())
}

fn prepare_deployment_command(deployment_name: &str, bundle: &Option<String>, version: &Option<String>) -> Result<()> {
    if version.is_some() {
        bail!("Deploying by version is not implemented yet");
    }

    let bundle_path_str = bundle.as_ref().context("Either --bundle or --version must be specified")?;
    let bundle_path = Path::new(bundle_path_str);

    // 1. Load specs
    let env_spec = spec_loader::load_env_spec(Path::new("."))?;
    let app_spec = spec_loader::load_app_spec(bundle_path, Some(&env_spec))?;

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
        },
        spec::DeploymentEnvType::Docker(docker_spec) => {
            let output_dir = Path::new("docker-deploy");
            docker_generator::generate(&resolved_spec, &docker_spec, output_dir).context("Generation failed")?;
            println!("Docker deployment script generated in {:?}", output_dir);
        },
        spec::DeploymentEnvType::Local => {
             bail!("prepare deployment doesn't support local deployments, use 'simpled local run' instead");
        }
    }

    Ok(())
}

fn local( command: &LocalCommands) -> Result<()> {
    let root = match command {
        LocalCommands::Run { path, .. } => path.as_ref()
            .map(|p| Path::new(p))
            .unwrap_or(Path::new(".")),
    };

    let env_spec = spec_loader::load_env_spec(root)?;
    let app_spec = spec_loader::load_app_spec_from_dir(Path::new("."), Some(&env_spec))?;

    let deployment = env_spec.deployments.first().context("No deployments defined in envspec")?;

    validator::validate(&env_spec, &app_spec, &deployment.name).context("Validation failed")?;

    println!("Validation passed for deployment {}", &deployment.name);

    // 3. Resolve
    let resolved_spec = resolver::resolve(&env_spec, &app_spec, &deployment.name).context("Resolution failed")?;

    // 4. Generate
    match env_spec.env_type {
        spec::DeploymentEnvType::K8S => {
            return Err(anyhow!("Environment type should be local"))
        },
        spec::DeploymentEnvType::Docker(_) => {
            return Err(anyhow!("Environment type should be local"))
        },
        spec::DeploymentEnvType::Local => {
            println!("Running local deployment");
            local_ingress::run(resolved_spec.ingress.clone())?;            
            run_local::run(&resolved_spec)?;
        },
    }

    Ok(())
}
