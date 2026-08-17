mod app;
mod env;

pub use app::convert_app_spec;
pub use env::convert_env_spec;

use anyhow::{Result, anyhow};
use crate::spec::{ServiceCommand, ServicePort, ServiceVolume, ServiceVolumeType};
use crate::spec_yaml::ServiceCommandYaml;

fn parse_ports(ports: &Option<Vec<String>>) -> Result<Vec<ServicePort>> {
    let Some(ports_yaml) = ports else {
        return Ok(Vec::new());
    };
    ports_yaml.iter().map(|s| {
        let (ext_str, int_str) = s.split_once(':')
            .ok_or_else(|| anyhow!("Invalid port format '{}'. Expected format 'external:internal'", s))?;
        let external = ext_str.parse::<u16>()
            .map_err(|_| anyhow!("Invalid external port '{}' in '{}'", ext_str, s))?;
        let internal = int_str.parse::<u16>()
            .map_err(|_| anyhow!("Invalid internal port '{}' in '{}'", int_str, s))?;
        Ok(ServicePort { external, internal })
    }).collect()
}

/// `"./host/path:/container/path"` or `"named:/container/path"`. A leading `.`
/// or `/` marks a host path; anything else is a named volume, which the app spec
/// must declare under its top-level `volumes:`.
fn parse_service_volume(s: &str) -> Result<ServiceVolume> {
    let (vol_str, mount_path) = s.split_once(':')
        .ok_or_else(|| anyhow!("Invalid volume format '{}'. Expected 'name:mount_path' or './path:mount_path'", s))?;

    let name = if vol_str.starts_with('.') || vol_str.starts_with('/') {
        ServiceVolumeType::Path(vol_str.to_string())
    } else {
        ServiceVolumeType::Named(vol_str.to_string())
    };

    Ok(ServiceVolume {
        name,
        mount_path: mount_path.to_string(),
    })
}

fn convert_service_command(c: ServiceCommandYaml) -> ServiceCommand {
    match c {
        ServiceCommandYaml::Shell(s) => ServiceCommand::Shell(s),
        ServiceCommandYaml::Exec(v) => ServiceCommand::Exec(v),
    }
}
