use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::fs::File;
use flate2::write::GzEncoder;
use flate2::Compression;

use crate::spec_loader;

pub fn create_app_bundle(
    registry: &Option<String>,
    push_images: bool,
    _upload: &Option<String>,
) -> Result<()> {
    let app_spec = spec_loader::load_app_spec_from_dir(Path::new("."), None)?;
    println!("Creating bundle for {} v{}", app_spec.name, app_spec.version);

    let mut registry_map = HashMap::new();
    if let Some(reg_str) = registry {
         for part in reg_str.split(',') {
             if let Some((k, v)) = part.split_once('=') {
                 registry_map.insert(k, v.to_string());
             }
         }
    }

    for service in app_spec.app_services {
        for variant in &service.image_variants {
             let source_image = &variant.image;
             let (base_name, _) = source_image.split_once(':').unwrap_or((source_image, ""));
             
             let mut target_image = format!("{}:{}", base_name, app_spec.version);
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

    let filename = format!("{}.{}.tar.gz", app_spec.name, app_spec.version);
    let file = File::create(&filename).context("Failed to create bundle file")?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = tar::Builder::new(enc);
    
    let path_yaml = Path::new("appspec.yaml");
    let path_yml = Path::new("appspec.yml");
    if path_yaml.exists() {
        tar.append_path("appspec.yaml")?;
    } else if path_yml.exists() {
        tar.append_path("appspec.yml")?;
    } else {
        bail!("appspec.yaml or appspec.yml not found");
    }


    tar.finish()?;
    
    println!("Created artifact: {}", filename);
    
    Ok(())
}
