use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use std::fs::File;
use std::path::Path;
use tar::Archive;

use crate::spec;
use crate::spec_yaml;
use crate::transform;

pub fn load_app_spec(
    app_bundle_path: &Path,
    env_spec: Option<&spec::DeploymentEnvironmentSpec>,
) -> Result<spec::AppSpec> {
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
                let yaml: spec_yaml::AppSpecYaml =
                    serde_yaml::from_reader(entry).context("Failed to parse appspec from tar.gz")?;
                return transform::convert_app_spec(yaml, env_spec).context("Failed to process app spec");
            }
        }
    }
    bail!("appspec.yaml not found in archive {:?}", path);
}

pub fn load_env_spec(root: &Path, selected_deployment: Option<&str>) -> Result<spec::DeploymentEnvironmentSpec> {
    let yaml = load_env_spec_yaml(root)?;
    transform::convert_env_spec(yaml, root, selected_deployment).context("Failed to process env spec")
}

/// The env spec as written, with `type` defaulted for a `localenv.yaml`, before
/// any conversion. `simpled test` adds a suite's inline deployment at this stage.
pub fn load_env_spec_yaml(root: &Path) -> Result<spec_yaml::DeploymentEnvironmentSpecYaml> {
    let candidates: &[(&str, bool)] = &[
        ("envspec.yaml", false),
        ("envspec.yml", false),
        ("localenv.yaml", true),
        ("localenv.yml", true),
    ];

    let (file_name, is_local_env) = candidates
        .iter()
        .find(|(name, _)| root.join(name).exists())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Could not find envspec.yaml, envspec.yml, localenv.yaml, or localenv.yml in {:?}",
                root
            )
        })?;

    let path = root.join(file_name);
    let file = File::open(&path).context(format!("Failed to open {:?}", path))?;
    let mut yaml: spec_yaml::DeploymentEnvironmentSpecYaml =
        serde_yaml::from_reader(file).context(format!("Failed to parse {:?}", path))?;

    if yaml.env_type.is_none() {
        if *is_local_env {
            yaml.env_type = Some(spec_yaml::DeploymentEnvTypeYaml::Local);
        } else {
            anyhow::bail!("'type' field is required in {:?}", path);
        }
    }

    Ok(yaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::fs;

    const APPSPEC: &str = "name: shop\nversion: 1.2.3\napp_services:\n  api:\n    image: myorg/api\n";

    #[test]
    fn an_app_spec_loads_from_a_directory_with_either_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("appspec.yml"), APPSPEC).unwrap();
        let spec = load_app_spec(dir.path(), None).unwrap();
        assert_eq!(spec.name, "shop");
        assert_eq!(spec.version.to_string(), "1.2.3");

        let empty = tempfile::tempdir().unwrap();
        let err = load_app_spec(empty.path(), None).unwrap_err().to_string();
        assert!(err.contains("Could not find appspec.yaml"), "{err}");
    }

    #[test]
    fn an_app_spec_loads_from_a_bundle_archive() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("shop.1.2.3.tar.gz");
        {
            let file = File::create(&bundle).unwrap();
            let mut tar = tar::Builder::new(GzEncoder::new(file, Compression::default()));
            let mut header = tar::Header::new_gnu();
            header.set_size(APPSPEC.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            // Nested inside a folder, as `tar czf` of a directory produces.
            tar.append_data(&mut header, "shop/appspec.yaml", APPSPEC.as_bytes())
                .unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let spec = load_app_spec(&bundle, None).unwrap();
        assert_eq!(spec.name, "shop");
        assert_eq!(spec.app_services[0].name, "api");
    }

    #[test]
    fn an_archive_without_an_app_spec_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("empty.tar.gz");
        {
            let file = File::create(&bundle).unwrap();
            let mut tar = tar::Builder::new(GzEncoder::new(file, Compression::default()));
            let mut header = tar::Header::new_gnu();
            header.set_size(2);
            header.set_cksum();
            tar.append_data(&mut header, "readme.txt", "hi".as_bytes()).unwrap();
            tar.into_inner().unwrap().finish().unwrap();
        }
        let err = load_app_spec(&bundle, None).unwrap_err().to_string();
        assert!(err.contains("appspec.yaml not found in archive"), "{err}");
    }

    #[test]
    fn a_bundle_must_be_a_directory_or_a_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("bundle.zip");
        fs::write(&odd, "").unwrap();
        let err = load_app_spec(&odd, None).unwrap_err().to_string();
        assert!(err.contains("directory or a tar.gz"), "{err}");
    }

    #[test]
    fn localenv_defaults_to_the_local_type_while_envspec_requires_one() {
        let dir = tempfile::tempdir().unwrap();
        let body = "gateway:\n  hosts:\n    web: localhost:8080\ndeployments:\n  dev:\n    primary_host: web\n    application:\n      name: shop\n";
        fs::write(dir.path().join("localenv.yaml"), body).unwrap();
        let spec = load_env_spec(dir.path(), None).unwrap();
        assert_eq!(spec.env_type, spec::DeploymentEnvType::Local);

        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("envspec.yaml"), body).unwrap();
        let err = load_env_spec(other.path(), None).unwrap_err().to_string();
        assert!(err.contains("'type' field is required"), "{err}");

        let none = tempfile::tempdir().unwrap();
        let err = load_env_spec(none.path(), None).unwrap_err().to_string();
        assert!(err.contains("Could not find envspec.yaml"), "{err}");
    }
}
