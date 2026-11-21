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

/// The app spec a local deployment runs: the appspec beside the env spec, or a
/// released bundle, whose app services then run the images published for it.
pub fn load_local_app_spec(
    root: &Path,
    env_spec: &mut spec::DeploymentEnvironmentSpec,
    deployment_name: &str,
    bundle: &crate::bundle_repo::BundleArgs,
) -> Result<spec::AppSpec> {
    let app_name = env_spec.deployment(deployment_name)?.application.name.clone();
    let Some(path) = bundle.locate(&app_name, &root.join(".simpled-bundles"))? else {
        return load_app_spec_from_dir(root, Some(env_spec));
    };
    let app_spec = load_app_spec(&path, Some(env_spec))?;
    env_spec.published_images = true;
    println!(
        "Running {} {} from its published images ({})",
        app_spec.name,
        app_spec.version,
        path.display()
    );
    Ok(app_spec)
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

pub fn load_env_spec(
    root: &Path,
    selected_deployment: Option<&str>,
    instance: u32,
) -> Result<spec::DeploymentEnvironmentSpec> {
    let mut yaml = load_env_spec_yaml(root)?;
    yaml.instance = instance;
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
    let text = std::fs::read_to_string(&path).context(format!("Failed to open {:?}", path))?;
    let mut yaml: spec_yaml::DeploymentEnvironmentSpecYaml =
        serde_yaml::from_str(&text).context(format!("Failed to parse {:?}", path))?;

    if yaml.env_type.is_none() {
        if *is_local_env {
            yaml.env_type = Some(spec_yaml::DeploymentEnvTypeYaml::Local);
        } else {
            anyhow::bail!("'type' field is required in {:?}", path);
        }
    }

    if yaml.deployments.values().any(|d| d.file.is_some()) {
        // As written: the typed deployment has already dropped the keys it does not know.
        let written: serde_yaml::Value = serde_yaml::from_str(&text).context(format!("Failed to parse {:?}", path))?;
        // In the order of their names, so the first error is the same on every run.
        let mut names: Vec<String> = yaml.deployments.keys().cloned().collect();
        names.sort();
        for name in names {
            let Some(file) = yaml.deployments[&name].file.clone() else {
                continue;
            };
            // Found by its name as a string. `2024:` or `true:` is another key to
            // YAML, and the fields beside `file` could not be checked.
            let loaded = match written["deployments"].get(name.as_str()) {
                Some(entry) => load_deployment_file(root, file_name, &name, entry, &file),
                None => Err(anyhow::anyhow!(
                    "a deployment read from a file needs a name YAML reads as a string: write it as \"{}\"",
                    name
                )),
            };
            let loaded = loaded.with_context(|| format!("Deployment '{}' in {:?}", name, path))?;
            yaml.deployments.insert(name, loaded);
        }
    }

    Ok(yaml)
}

/// The deployment a `file:` entry stands for. The file sits next to the env spec,
/// never in a subdirectory, so that the paths written in it resolve from the same
/// directory whichever of the two files they are in.
fn load_deployment_file(
    root: &Path,
    env_spec_name: &str,
    deployment_name: &str,
    entry: &serde_yaml::Value,
    file: &str,
) -> Result<spec_yaml::DeploymentSpecYaml> {
    if let serde_yaml::Value::Mapping(fields) = entry {
        let mut others: Vec<String> = fields
            .keys()
            .filter(|key| key.as_str() != Some("file"))
            .map(|key| match key.as_str() {
                Some(key) => key.to_string(),
                None => serde_yaml::to_string(key).unwrap_or_default().trim().to_string(),
            })
            .collect();
        others.sort();
        if !others.is_empty() {
            bail!(
                "`file: {}` cannot be combined with other fields ({}): move them into the file",
                file,
                others.join(", ")
            );
        }
    }

    let name = file.strip_prefix("./").unwrap_or(file);
    let is_bare_name = !name.is_empty()
        && !name.contains(['/', '\\'])
        && matches!(
            Path::new(name).components().collect::<Vec<_>>().as_slice(),
            [std::path::Component::Normal(_)]
        );
    if !is_bare_name {
        bail!(
            "`file: {}` must name a file in the same directory as {}: subdirectories and other paths are not allowed",
            file,
            env_spec_name
        );
    }
    // Ignoring case, as Windows and macOS open `Envspec.yaml` as `envspec.yaml`.
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".yaml") && !lower.ends_with(".yml") {
        bail!("`file: {}` must be a .yaml or .yml file", file);
    }
    if ["envspec.yaml", "envspec.yml", "localenv.yaml", "localenv.yml"].contains(&lower.as_str()) {
        bail!(
            "`file: {}` is an env spec's own name; give the deployment's file another",
            file
        );
    }

    let path = root.join(name);
    let text = std::fs::read_to_string(&path).with_context(|| format!("Failed to open {:?}", path))?;
    let mut deployment: spec_yaml::DeploymentSpecYaml =
        serde_yaml::from_str(&text).with_context(|| format!("Failed to parse {:?}", path))?;

    // A deployment ignores the keys it does not know, so a file holding the
    // `deployments:` block, or the deployment under its name, would load empty.
    let known = serde_yaml::to_value(serde_yaml::from_str::<spec_yaml::DeploymentSpecYaml>("{}")?)?;
    if let Ok(serde_yaml::Value::Mapping(written)) = serde_yaml::from_str(&text) {
        let mut unknown: Vec<String> = written
            .keys()
            .filter(|key| key.as_str().is_none_or(|k| known.get(k).is_none()))
            .map(|key| match key.as_str() {
                Some(key) => key.to_string(),
                None => serde_yaml::to_string(key).unwrap_or_default().trim().to_string(),
            })
            .collect();
        unknown.sort();
        if !unknown.is_empty() {
            bail!(
                "{:?} has {} a deployment does not have: {}. The file holds the fields of deployment '{}' themselves \
                 (primary_host, application, ...), as they would be written under its name in {}",
                path,
                if unknown.len() == 1 { "a key" } else { "keys" },
                unknown.join(", "),
                deployment_name,
                env_spec_name
            );
        }
    }
    if deployment.file.is_some() {
        bail!(
            "{:?} sets `file` itself: it holds deployment '{}', written out",
            path,
            deployment_name
        );
    }
    // Kept, so a message about the deployment can say where it is written.
    deployment.file = Some(name.to_string());
    Ok(deployment)
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
        let spec = load_env_spec(dir.path(), None, 0).unwrap();
        assert_eq!(spec.env_type, spec::DeploymentEnvType::Local);

        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("envspec.yaml"), body).unwrap();
        let err = load_env_spec(other.path(), None, 0).unwrap_err().to_string();
        assert!(err.contains("'type' field is required"), "{err}");

        let none = tempfile::tempdir().unwrap();
        let err = load_env_spec(none.path(), None, 0).unwrap_err().to_string();
        assert!(err.contains("Could not find envspec.yaml"), "{err}");
    }

    const SPLIT_HEAD: &str = "gateway:\n  hosts:\n    web: localhost:8080\n    admin: localhost:8081\ndeployments:\n";
    const SHOP: &str = "primary_host: web\napplication:\n  name: shop\nenvironment:\n  - A=1\n";

    fn split_env(deployments: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("localenv.yaml"), format!("{SPLIT_HEAD}{deployments}")).unwrap();
        for (name, body) in files {
            let path = dir.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        dir
    }

    fn split_error(deployments: &str, files: &[(&str, &str)]) -> String {
        let dir = split_env(deployments, files);
        format!("{:#}", load_env_spec(dir.path(), None, 0).unwrap_err())
    }

    #[test]
    fn a_deployment_is_read_from_a_file_next_to_the_env_spec() {
        let dir = split_env(
            "  shop:\n    file: ./shop.yaml\n  staging:\n    file: staging.yml\n  admin:\n    primary_host: admin\n    application:\n      name: admin\n",
            &[
                ("shop.yaml", SHOP),
                // Deployments in files extend each other by name, like inline ones.
                ("staging.yml", "extends: shop\nenvironment:\n  - B=2\n"),
            ],
        );
        let yaml = load_env_spec_yaml(dir.path()).unwrap();
        assert_eq!(yaml.deployments["shop"].file.as_deref(), Some("shop.yaml"));
        assert_eq!(yaml.deployments["admin"].file, None);

        let spec = load_env_spec(dir.path(), Some("staging"), 0).unwrap();
        let staging = spec.deployment("staging").unwrap();
        assert_eq!(staging.file.as_deref(), Some("staging.yml"));
        assert_eq!(staging.application.name, "shop");
        let names: Vec<&str> = staging.environment.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        assert_eq!(
            load_env_spec(dir.path(), Some("admin"), 0)
                .unwrap()
                .deployment("admin")
                .unwrap()
                .application
                .name,
            "admin"
        );
    }

    #[test]
    fn a_deployment_file_is_never_in_another_directory() {
        for file in [
            "deployments/shop.yaml",
            "./deployments/shop.yaml",
            "../shop.yaml",
            "deployments\\\\shop.yaml",
            "/etc/shop.yaml",
            "..",
            "./",
        ] {
            let err = split_error(
                &format!("  shop:\n    file: \"{file}\"\n"),
                &[("deployments/shop.yaml", SHOP)],
            );
            assert!(err.contains("same directory as localenv.yaml"), "{file}: {err}");
            assert!(err.contains("Deployment 'shop'"), "{file}: {err}");
        }
    }

    #[test]
    fn a_deployment_file_entry_is_checked() {
        let err = split_error(
            "  shop:\n    file: shop.yaml\n    primary_host: web\n    environment:\n      - A=1\n",
            &[("shop.yaml", SHOP)],
        );
        assert!(
            err.contains("cannot be combined with other fields (environment, primary_host)"),
            "{err}"
        );

        // Keys a deployment does not have are not dropped unnoticed either.
        let err = split_error(
            "  shop:\n    file: shop.yaml\n    primary_hots: web\n    replicas: 2\n",
            &[("shop.yaml", SHOP)],
        );
        assert!(err.contains("other fields (primary_hots, replicas)"), "{err}");

        // A name YAML does not read as a string would escape that check.
        for name in ["2024", "true", "0x10"] {
            let err = split_error(
                &format!("  {name}:\n    file: shop.yaml\n    primary_host: web\n"),
                &[("shop.yaml", SHOP)],
            );
            assert!(err.contains(&format!("write it as \"{name}\"")), "{name}: {err}");
        }
        let quoted = split_env("  \"2024\":\n    file: shop.yaml\n", &[("shop.yaml", SHOP)]);
        assert!(load_env_spec(quoted.path(), None, 0).is_ok());

        // Several broken files: always the first by deployment name.
        for _ in 0..8 {
            let err = split_error(
                "  shop:\n    file: shop.yaml\n  admin:\n    file: admin.yaml\n  jobs:\n    file: jobs.yaml\n",
                &[],
            );
            assert!(err.contains("Deployment 'admin'"), "{err}");
        }

        let err = split_error("  shop:\n    file: shop.json\n", &[]);
        assert!(err.contains("must be a .yaml or .yml file"), "{err}");

        for own in ["localenv.yaml", "Envspec.YML"] {
            let err = split_error(&format!("  shop:\n    file: {own}\n"), &[]);
            assert!(err.contains("env spec's own name"), "{own}: {err}");
        }

        let err = split_error("  shop:\n    file: shop.yaml\n", &[]);
        assert!(err.contains("Failed to open") && err.contains("shop.yaml"), "{err}");

        let err = split_error(
            "  shop:\n    file: shop.yaml\n",
            &[("shop.yaml", "primary_host: [web\n")],
        );
        assert!(err.contains("Failed to parse") && err.contains("shop.yaml"), "{err}");

        let err = split_error(
            "  shop:\n    file: shop.yaml\n",
            &[("shop.yaml", "file: other.yaml\n"), ("other.yaml", SHOP)],
        );
        assert!(err.contains("sets `file` itself"), "{err}");
    }

    #[test]
    fn a_deployment_file_holds_only_the_fields_of_a_deployment() {
        // The deployment under its name, a whole env spec, and a field that is
        // half moved: each would otherwise load with those keys dropped.
        let indented: String = SHOP.lines().map(|l| format!("  {l}\n")).collect();
        for (body, keys) in [
            (format!("shop:\n{indented}"), "a key a deployment does not have: shop."),
            (
                format!("type: local\ngateway:\n  hosts: {{}}\ndeployments:\n  shop:\n  {indented}"),
                "keys a deployment does not have: deployments, gateway, type.",
            ),
            (
                format!("{SHOP}replicas: 2\n"),
                "a key a deployment does not have: replicas.",
            ),
        ] {
            let err = split_error("  shop:\n    file: shop.yaml\n", &[("shop.yaml", &body)]);
            assert!(err.contains(keys), "{body}\n{err}");
            assert!(err.contains("shop.yaml") && err.contains("in localenv.yaml"), "{err}");
        }

        // Every field a deployment has is accepted, `abstract` by its written name.
        let dir = split_env(
            "  base:\n    file: base.yaml\n  shop:\n    extends: base\n",
            &[(
                "base.yaml",
                "abstract: true\nprimary_host: web\napplication:\n  name: shop\nservices: {}\nsecrets: {}\n",
            )],
        );
        assert!(load_env_spec(dir.path(), Some("shop"), 0).is_ok());
    }
}
