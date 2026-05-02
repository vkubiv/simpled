mod app;
mod env;

pub use app::convert_app_spec;
pub use env::convert_env_spec;

use anyhow::{Result, anyhow};
use crate::spec::ServicePort;

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
