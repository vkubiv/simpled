use anyhow::{Context, Result, bail};
use std::fs::File;
use std::path::Path;
use flate2::read::GzDecoder;
use tar::Archive;

use crate::spec;
use crate::spec_yaml;
use crate::transform;

pub fn load_app_spec(app_bundle_path: &Path, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<spec::AppSpec> {
    if app_bundle_path.is_dir() {
        return load_app_spec_from_dir(app_bundle_path, env_spec);
    } else if let Some(ext) = app_bundle_path.extension() {
        if ext == "gz" {
            return load_app_spec_from_tar_gz(app_bundle_path, env_spec);
        }
    }

    bail!("Invalid app bundle can be either a directory or a tar.gz file");
}

pub fn load_app_spec_from_dir(dir: &Path, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<spec::AppSpec> {
    let path_yaml = dir.join("appspec.yaml");
    let path_yml = dir.join("appspec.yml");

    let path = if path_yaml.exists() {
        path_yaml
    } else if path_yml.exists() {
        path_yml
    } else {
        bail!("Could not find appspec.yaml or appspec.yml in {:?}", dir);
    };

    load_app_spec_from_file(&path, env_spec)
}

fn load_app_spec_from_file(path: &Path, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<spec::AppSpec> {
    let file = File::open(path).context(format!("Failed to open {:?}", path))?;
    let yaml: spec_yaml::AppSpecYaml = serde_yaml::from_reader(file).context(format!("Failed to parse {:?}", path))?;
    transform::convert_app_spec(yaml, env_spec).context("Failed to process app spec")
}

fn load_app_spec_from_tar_gz(path: &Path, env_spec: Option<&spec::DeploymentEnvironmentSpec>) -> Result<spec::AppSpec> {
    let file = File::open(path).context(format!("Failed to open {:?}", path))?;
    let tar = GzDecoder::new(file);
    let mut archive = Archive::new(tar);

    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        if let Some(name) = path.file_name() {
             if name == "appspec.yaml" || name == "appspec.yml" {
                  let yaml: spec_yaml::AppSpecYaml = serde_yaml::from_reader(entry).context("Failed to parse appspec from tar.gz")?;
                  return transform::convert_app_spec(yaml, env_spec).context("Failed to process app spec");
             }
        }
    }
    bail!("appspec.yaml not found in archive {:?}", path);
}

pub fn load_env_spec(root: &Path) -> Result<spec::DeploymentEnvironmentSpec> {
    let path = root.join(Path::new("envspec.yaml"));

    let file = if path.exists() {
        File::open(path)?
    } else {
        File::open("envspec.yml").context("Could not find envspec.yaml or envspec.yml")?
    };

    let yaml: spec_yaml::DeploymentEnvironmentSpecYaml = serde_yaml::from_reader(file).context("Failed to parse envspec.yaml")?;
    let env_spec = transform::convert_env_spec(yaml, root).context("Failed to process env spec")?;
    Ok(env_spec)
}
