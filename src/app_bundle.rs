use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::fs::{self, File};
use std::time::{SystemTime, UNIX_EPOCH};
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::spec::{self, ImageSpec};
use crate::spec_loader;
use crate::bundle_repo;

/// Turns an arbitrary label into a valid semver build-metadata identifier so that a
/// raw branch name can be passed straight through from CI.
///
/// Everything outside `[A-Za-z0-9]` becomes `-`, runs of `-` collapse into one, and
/// leading/trailing `-` are dropped: `feature/big refactor` gives `feature-big-refactor`.
/// Returns an empty string when nothing usable is left.
fn sanitize_suffix(suffix: &str) -> String {
    let mut out = String::with_capacity(suffix.len());
    for ch in suffix.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }

    out.trim_matches('-').to_string()
}

/// Appends `suffix` to `version` as semver build metadata: `1.0.2` + `big-refactor`
/// gives `1.0.2+big-refactor`.
///
/// Build metadata is the right slot for a side-branch label: it is ignored when
/// versions are compared, so a `version: "^1.0"` requirement in an envspec still
/// accepts the bundle. When the version already carries build metadata, the suffix
/// becomes another dot-separated identifier.
fn apply_version_suffix(version: &semver::Version, suffix: &str) -> Result<semver::Version> {
    let sanitized = sanitize_suffix(suffix);
    if sanitized.is_empty() {
        bail!("--version-suffix '{}' contains no alphanumeric characters", suffix);
    }
    if sanitized != suffix {
        println!("Version suffix '{}' normalized to '{}'", suffix, sanitized);
    }

    let build_str = if version.build.is_empty() {
        sanitized.clone()
    } else {
        format!("{}.{}", version.build, sanitized)
    };

    let build = semver::BuildMetadata::new(&build_str)
        .with_context(|| format!("Invalid version suffix '{}'", sanitized))?;

    let mut version = version.clone();
    version.build = build;
    Ok(version)
}

/// Replaces the top-level `version:` field of an appspec with `version`.
///
/// Done textually rather than by a serde round-trip so comments, key order and
/// fields the parser doesn't model are preserved.
fn rewrite_version(content: &str, version: &semver::Version) -> Result<String> {
    let mut out = String::with_capacity(content.len() + 32);
    let mut replaced = false;

    for line in content.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);

        // A top-level key sits at column 0, so no indent check is needed.
        if !replaced && body.starts_with("version:") {
            let value = &body["version:".len()..];
            let comment = value.find(" #").map(|i| &value[i..]).unwrap_or("");
            out.push_str(&format!("version: {}{}", version, comment));
            out.push_str(&line[body.len()..]);
            replaced = true;
        } else {
            out.push_str(line);
        }
    }

    if !replaced {
        bail!("Could not find a top-level 'version:' field in the appspec to apply the version suffix to");
    }

    Ok(out)
}

pub fn create_app_bundle(
    registry: &Option<String>,
    push_images: bool,
    _upload: &Option<String>,
    upload_bundle_to: &Option<String>,
    gh_repo: &Option<String>,
    gh_tag_prefix: &Option<String>,
    version_suffix: &Option<String>,
) -> Result<()> {
    let app_spec = spec_loader::load_app_spec_from_dir(Path::new("."), None)?;

    let version = match version_suffix {
        Some(suffix) => apply_version_suffix(&app_spec.version, suffix)?,
        None => app_spec.version.clone(),
    };
    // Image tags, the bundle file name and the release tag cannot carry the `+` that
    // separates build metadata, so they all use the `-` form of the same version.
    let version_tag = spec::version_to_tag(&version.to_string());

    println!("Creating bundle for {} v{}", app_spec.name, version);

    let mut registry_map = HashMap::new();
    if let Some(reg_str) = registry {
         for part in reg_str.split(',') {
             if let Some((k, v)) = part.split_once('=') {
                 registry_map.insert(k, v.to_string());
             }
         }
    }

    for service in app_spec.app_services {
        let images: Vec<String> = match service.image {
            ImageSpec::Exact(img) => vec![img],
            ImageSpec::Variants(variants) => variants.into_iter().map(|v| v.image).collect(),
        };
        for source_image in &images {
             let (base_name, _) = source_image.split_once(':').unwrap_or((source_image, ""));
             
             let mut target_image = format!("{}:{}", base_name, version_tag);
             let mut matched = false;

             for (prefix, reg_url) in &registry_map {
                 let mut prefix = prefix.to_string();
                 if !prefix.ends_with('/') {
                     prefix.push('/');
                 }
                 
                 if base_name.starts_with(&prefix) {
                     let url = reg_url.trim_end_matches('/');
                     target_image = format!("{}/{}", url, target_image);
                     matched = true;
                     break; 
                 }
             }
             
             if !registry_map.is_empty() && !matched {
                 let available = registry_map
                     .iter()
                     .map(|(k, v)| format!("{}={}", k, v))
                     .collect::<Vec<_>>()
                     .join(", ");
                 bail!("No registry match for image {}, available registries: {}", source_image, available);
             }

             println!("Tagging {} as {}", source_image, target_image);
             
             let status = Command::new("docker")
                 .arg("tag")
                 .arg(source_image)
                 .arg(&target_image)
                 .status()
                 .with_context(|| format!("Failed to execute docker tag for {}", source_image))?;
                 
             if !status.success() {
                 bail!("Docker tag failed for {}", source_image);
             }
             
             if push_images {
                 println!("Pushing {}", target_image);
                 let status = Command::new("docker")
                     .arg("push")
                     .arg(&target_image)
                     .status()
                     .with_context(|| format!("Failed to execute docker push for {}", target_image))?;
                     
                 if !status.success() {
                     bail!("Docker push failed for {}", target_image);
                 }
             }
        }
    }

    let filename = format!("{}.{}.tar.gz", app_spec.name, version_tag);
    let file = File::create(&filename).context("Failed to create bundle file")?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);

    let path_yaml = Path::new("appspec.yaml");
    let path_yml = Path::new("appspec.yml");
    let spec_path = if path_yaml.exists() {
        path_yaml
    } else if path_yml.exists() {
        path_yml
    } else {
        bail!("appspec.yaml or appspec.yml not found");
    };

    if version_suffix.is_some() {
        // The suffixed version has to travel inside the bundle: `prepare-deployment`
        // reads the version from the bundled appspec to tag the images it deploys,
        // so an untouched appspec would point the deployment at unsuffixed images.
        let content = fs::read_to_string(spec_path)
            .with_context(|| format!("Failed to read {:?}", spec_path))?;
        let patched = rewrite_version(&content, &version)?;

        let mtime = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut header = tar::Header::new_gnu();
        header.set_size(patched.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(mtime);
        header.set_cksum();
        tar.append_data(&mut header, spec_path, patched.as_bytes())?;
    } else {
        tar.append_path(spec_path)?;
    }


    let archive = tar.into_inner().context("Failed to write bundle file")?;

    archive.finish().context("Failed to finish bundle file")?;
    
    println!("Created artifact: {}", filename);
    
    if let Some(target) = upload_bundle_to {
        if target == "github-release" {
            let repo = gh_repo.as_ref().context("--github-repo is required when uploading to github-release")?;
            bundle_repo::gh_release::upload(repo, &version_tag, &filename, gh_tag_prefix.as_deref())?;
        } else {
            bail!("Unknown upload target: {}. Only 'github-release' is supported.", target);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffixed(version: &str, suffix: &str) -> String {
        apply_version_suffix(&semver::Version::parse(version).unwrap(), suffix)
            .unwrap()
            .to_string()
    }

    #[test]
    fn test_sanitize_suffix() {
        // Already valid identifiers pass through untouched.
        assert_eq!(sanitize_suffix("big-refactor"), "big-refactor");
        assert_eq!(sanitize_suffix("JIRA123"), "JIRA123");
        assert_eq!(sanitize_suffix("007"), "007");

        // Branch names get their invalid characters replaced with '-'.
        assert_eq!(sanitize_suffix("feature/big-refactor"), "feature-big-refactor");
        assert_eq!(sanitize_suffix("feature/big refactor"), "feature-big-refactor");
        assert_eq!(sanitize_suffix("user@host_branch.v2"), "user-host-branch-v2");

        // Runs collapse, leading/trailing separators are dropped.
        assert_eq!(sanitize_suffix("a///b"), "a-b");
        assert_eq!(sanitize_suffix("-big-refactor-"), "big-refactor");
        assert_eq!(sanitize_suffix("/feature/"), "feature");

        // Nothing usable left.
        assert_eq!(sanitize_suffix("///"), "");
        assert_eq!(sanitize_suffix(""), "");
    }

    #[test]
    fn test_apply_version_suffix() {
        assert_eq!(suffixed("1.0.2", "big-refactor"), "1.0.2+big-refactor");
        assert_eq!(suffixed("1.0.2", "feature/big-refactor"), "1.0.2+feature-big-refactor");

        // A pre-release is preserved alongside the suffix.
        assert_eq!(suffixed("1.0.2-rc1", "big-refactor"), "1.0.2-rc1+big-refactor");

        // Existing build metadata is kept and extended.
        assert_eq!(suffixed("1.0.2+abc", "big-refactor"), "1.0.2+abc.big-refactor");

        let version = semver::Version::parse("1.0.2").unwrap();
        assert!(apply_version_suffix(&version, "///").is_err());
    }

    #[test]
    fn test_suffixed_version_still_satisfies_requirement() {
        // The reason the suffix lives in build metadata: semver ignores it when
        // matching, so an envspec requirement keeps accepting side-branch bundles.
        let version = semver::Version::parse("1.0.2").unwrap();
        let suffixed = apply_version_suffix(&version, "big-refactor").unwrap();

        assert!(semver::VersionReq::parse("^1.0").unwrap().matches(&suffixed));
        assert!(semver::VersionReq::parse("=1.0.2").unwrap().matches(&suffixed));
        assert!(!semver::VersionReq::parse("^2.0").unwrap().matches(&suffixed));
    }

    #[test]
    fn test_version_to_tag() {
        // `+` is not legal in a docker tag, so image tags, bundle names and release
        // tags use the `-` form.
        assert_eq!(spec::version_to_tag("1.0.2+big-refactor"), "1.0.2-big-refactor");
        assert_eq!(spec::version_to_tag("1.0.2-rc1+big-refactor"), "1.0.2-rc1-big-refactor");
        assert_eq!(spec::version_to_tag("1.0.2"), "1.0.2");
    }

    #[test]
    fn test_rewrite_version() {
        let version = semver::Version::parse("1.0.2+big-refactor").unwrap();

        let spec = "name: myapp\nversion: 1.0.2\napp_services:\n  api:\n    version: ignored\n";
        assert_eq!(
            rewrite_version(spec, &version).unwrap(),
            "name: myapp\nversion: 1.0.2+big-refactor\napp_services:\n  api:\n    version: ignored\n"
        );

        // Quoted values, inline comments and CRLF line endings survive.
        let spec = "version: \"1.0.2\" # keep me\r\nname: myapp\r\n";
        assert_eq!(
            rewrite_version(spec, &version).unwrap(),
            "version: 1.0.2+big-refactor # keep me\r\nname: myapp\r\n"
        );

        // No trailing newline on the last line.
        assert_eq!(
            rewrite_version("name: myapp\nversion: 1.0.2", &version).unwrap(),
            "name: myapp\nversion: 1.0.2+big-refactor"
        );

        assert!(rewrite_version("name: myapp\n", &version).is_err());
    }
}
