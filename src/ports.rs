//! Named host ports and the instance offset, so several copies of one local
//! stack can run on a machine side by side (two worktrees, or a test run next
//! to the developer's own stack).
//!
//! `localenv.yaml` names its host ports once, and refers to them as
//! `$port(name)` anywhere a string is written. An instance shifts every named
//! port by `instance * port_step`, and gives the compose project and output
//! directory a suffix of its own. Instance 0 is the stack exactly as written.

use anyhow::{anyhow, bail, Context, Result};
use serde_yaml::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::TcpListener;
use std::path::Path;

pub const INSTANCE_ENV: &str = "SIMPLED_INSTANCE";
pub const INSTANCE_FILE: &str = ".simpled-instance";
const MAX_AUTO_INSTANCE: u32 = 50;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalPorts {
    instance: u32,
    step: u32,
    base: BTreeMap<String, u16>,
}

impl LocalPorts {
    pub fn new(base: BTreeMap<String, u16>, step: Option<u32>, instance: u32) -> Result<Self> {
        for name in base.keys() {
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                bail!("Port name '{}' may only use letters, digits, '_' and '-'", name);
            }
        }

        let span = match (base.values().min(), base.values().max()) {
            (Some(min), Some(max)) => (*max - *min) as u32,
            _ => 0,
        };
        // A step wider than the spread of the named ports keeps every instance's
        // ports apart from every other instance's.
        let step = match step {
            Some(step) if step <= span => bail!(
                "port_step {} must be larger than the spread of the named ports ({}), or one instance's ports land on another's",
                step,
                span
            ),
            Some(step) => step,
            None => (span / 100 + 1) * 100,
        };

        if instance > 0 && base.is_empty() {
            bail!(
                "Instance {} needs named ports: declare `ports:` in localenv.yaml and use $port(name) where a host port is written",
                instance
            );
        }

        let ports = LocalPorts { instance, step, base };
        for (name, port) in &ports.base {
            let shifted = *port as u64 + instance as u64 * step as u64;
            if shifted > u16::MAX as u64 {
                bail!(
                    "Instance {} puts port '{}' ({}) at {}, past 65535",
                    instance,
                    name,
                    port,
                    shifted
                );
            }
        }
        Ok(ports)
    }

    pub fn with_instance(&self, instance: u32) -> Result<Self> {
        LocalPorts::new(self.base.clone(), Some(self.step), instance)
    }

    pub fn instance(&self) -> u32 {
        self.instance
    }

    /// `""` for instance 0, `"_<n>"` otherwise: what compose project and output
    /// directory names take, so instance 0 keeps today's names.
    pub fn suffix(&self) -> String {
        if self.instance == 0 {
            String::new()
        } else {
            format!("_{}", self.instance)
        }
    }

    pub fn port(&self, name: &str) -> Result<u16> {
        let base = self.base.get(name).ok_or_else(|| {
            let names: Vec<&str> = self.base.keys().map(String::as_str).collect();
            if names.is_empty() {
                anyhow!("$port({}) used, but localenv.yaml declares no `ports:`", name)
            } else {
                anyhow!("$port({}) names no port. Declared ports: {}", name, names.join(", "))
            }
        })?;
        Ok((*base as u32 + self.instance * self.step) as u16)
    }

    /// Every named port as this instance publishes it.
    pub fn effective(&self) -> BTreeSet<u16> {
        self.base
            .keys()
            .map(|name| self.port(name).expect("declared port"))
            .collect()
    }

    /// Expands `$port(name)` and `$instance()` in `input`.
    pub fn expand(&self, input: &str) -> Result<String> {
        let mut out = String::with_capacity(input.len());
        let mut rest = input;
        while let Some(pos) = rest.find('$') {
            out.push_str(&rest[..pos]);
            let tail = &rest[pos..];
            if let Some(after) = tail.strip_prefix("$port(") {
                let end = after
                    .find(')')
                    .ok_or_else(|| anyhow!("Unclosed $port( in '{}'", input))?;
                out.push_str(&self.port(after[..end].trim())?.to_string());
                rest = &after[end + 1..];
            } else if let Some(after) = tail.strip_prefix("$instance()") {
                out.push_str(&self.instance.to_string());
                rest = after;
            } else {
                out.push('$');
                rest = &tail[1..];
            }
        }
        out.push_str(rest);
        Ok(out)
    }

    /// Expands every string in a YAML document, keys included.
    pub fn expand_yaml(&self, value: &mut Value) -> Result<()> {
        match value {
            Value::String(s) => *s = self.expand(s)?,
            Value::Sequence(items) => {
                for item in items {
                    self.expand_yaml(item)?;
                }
            }
            Value::Mapping(map) => {
                let entries = std::mem::take(map);
                for (mut key, mut val) in entries {
                    self.expand_yaml(&mut key)?;
                    self.expand_yaml(&mut val)?;
                    map.insert(key, val);
                }
            }
            Value::Tagged(tagged) => self.expand_yaml(&mut tagged.value)?,
            _ => {}
        }
        Ok(())
    }

    /// The lowest instance whose every named port can be bound on this machine.
    pub fn first_free_instance(&self) -> Result<u32> {
        if self.base.is_empty() {
            bail!("--instance auto needs named ports: declare `ports:` in localenv.yaml");
        }
        for instance in 0..=MAX_AUTO_INSTANCE {
            let Ok(candidate) = self.with_instance(instance) else {
                break;
            };
            if candidate.effective().iter().all(|p| port_is_free(*p)) {
                return Ok(instance);
            }
        }
        bail!("No instance up to {} has all its named ports free", MAX_AUTO_INSTANCE)
    }
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok() && TcpListener::bind(("0.0.0.0", port)).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstanceChoice {
    Fixed(u32),
    Auto,
}

/// Which instance to run: the `--instance` flag, then `SIMPLED_INSTANCE`, then
/// a `.simpled-instance` file next to the env spec, then 0.
pub fn choose_instance(root: &Path, flag: Option<&str>) -> Result<InstanceChoice> {
    if let Some(raw) = flag {
        return parse_instance(raw).context("--instance");
    }
    if let Ok(raw) = std::env::var(INSTANCE_ENV) {
        if !raw.trim().is_empty() {
            return parse_instance(&raw).with_context(|| INSTANCE_ENV.to_string());
        }
    }
    let file = root.join(INSTANCE_FILE);
    if file.exists() {
        let raw = fs::read_to_string(&file).with_context(|| format!("Failed to read {:?}", file))?;
        return parse_instance(&raw).with_context(|| format!("{:?}", file));
    }
    Ok(InstanceChoice::Fixed(0))
}

fn parse_instance(raw: &str) -> Result<InstanceChoice> {
    let raw = raw.trim();
    if raw == "auto" {
        return Ok(InstanceChoice::Auto);
    }
    raw.parse::<u32>()
        .map(InstanceChoice::Fixed)
        .map_err(|_| anyhow!("'{}' is not an instance number or 'auto'", raw))
}

/// The named ports a `localenv.yaml` declares, read before the rest of it, so an
/// `auto` instance can be picked before anything is expanded.
pub fn read_declared(root: &Path) -> Result<LocalPorts> {
    let Some(path) = ["localenv.yaml", "localenv.yml"]
        .iter()
        .map(|n| root.join(n))
        .find(|p| p.exists())
    else {
        return Ok(LocalPorts::default());
    };
    let content = fs::read_to_string(&path).with_context(|| format!("Failed to read {:?}", path))?;
    let value: Value = serde_yaml::from_str(&content).with_context(|| format!("Failed to parse {:?}", path))?;
    declared_in(&value, 0)
}

/// The `ports:` and `port_step:` of an env spec document, for `instance`.
pub fn declared_in(value: &Value, instance: u32) -> Result<LocalPorts> {
    let base: BTreeMap<String, u16> = match value.get("ports") {
        Some(ports) => serde_yaml::from_value(ports.clone()).context("`ports:` must map names to port numbers")?,
        None => BTreeMap::new(),
    };
    let step: Option<u32> = match value.get("port_step") {
        Some(step) => Some(serde_yaml::from_value(step.clone()).context("`port_step:` must be a number")?),
        None => None,
    };
    LocalPorts::new(base, step, instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ports(instance: u32) -> LocalPorts {
        let base = BTreeMap::from([
            ("web".to_string(), 4000),
            ("api".to_string(), 4001),
            ("db".to_string(), 5432),
        ]);
        LocalPorts::new(base, None, instance).unwrap()
    }

    #[test]
    fn instance_zero_is_the_spec_as_written() {
        let p = ports(0);
        assert_eq!(p.expand("localhost:$port(api)/x").unwrap(), "localhost:4001/x");
        assert_eq!(p.suffix(), "");
    }

    #[test]
    fn an_instance_shifts_every_port_by_the_default_step() {
        // Spread 1432 rounds up to a 1500 step.
        let p = ports(2);
        assert_eq!(p.expand("$port(web):80 $port(db)").unwrap(), "7000:80 8432");
        assert_eq!(p.expand("id-$instance()").unwrap(), "id-2");
        assert_eq!(p.suffix(), "_2");
    }

    #[test]
    fn other_dollar_references_are_left_alone() {
        let p = ports(1);
        assert_eq!(
            p.expand("postgres://u:$secret(pw)@db:$port(db) ${VAR} $").unwrap(),
            "postgres://u:$secret(pw)@db:6932 ${VAR} $"
        );
    }

    #[test]
    fn unknown_and_unclosed_references_are_errors() {
        let p = ports(0);
        assert!(p
            .expand("$port(nope)")
            .unwrap_err()
            .to_string()
            .contains("Declared ports: api, db, web"));
        assert!(p.expand("$port(api").unwrap_err().to_string().contains("Unclosed"));
    }

    #[test]
    fn a_step_that_lets_instances_overlap_is_rejected() {
        let base = BTreeMap::from([("a".to_string(), 4000), ("b".to_string(), 4200)]);
        let err = LocalPorts::new(base, Some(100), 0).unwrap_err().to_string();
        assert!(err.contains("larger than the spread"), "{err}");
    }

    #[test]
    fn an_instance_without_named_ports_is_rejected() {
        let err = LocalPorts::new(BTreeMap::new(), None, 1).unwrap_err().to_string();
        assert!(err.contains("needs named ports"), "{err}");
        assert!(LocalPorts::new(BTreeMap::new(), None, 0).is_ok());
    }

    #[test]
    fn a_shift_past_the_port_range_is_rejected() {
        let base = BTreeMap::from([("a".to_string(), 60000)]);
        let err = LocalPorts::new(base, Some(10000), 1).unwrap_err().to_string();
        assert!(err.contains("past 65535"), "{err}");
    }

    #[test]
    fn yaml_strings_are_expanded_throughout() {
        let mut doc: Value = serde_yaml::from_str("hosts:\n  web: [\"localhost:$port(web)\"]\nn: 3\n").unwrap();
        ports(1).expand_yaml(&mut doc).unwrap();
        assert_eq!(doc["hosts"]["web"][0].as_str(), Some("localhost:5500"));
        assert_eq!(doc["n"].as_u64(), Some(3));
    }

    #[test]
    fn declared_ports_are_read_from_the_document() {
        let doc: Value = serde_yaml::from_str("ports:\n  a: 4000\n  b: 4010\nport_step: 50\n").unwrap();
        let p = declared_in(&doc, 2).unwrap();
        assert_eq!(p.port("b").unwrap(), 4110);
    }

    #[test]
    fn instance_choice_parses_numbers_and_auto() {
        assert_eq!(parse_instance(" 3\n").unwrap(), InstanceChoice::Fixed(3));
        assert_eq!(parse_instance("auto").unwrap(), InstanceChoice::Auto);
        assert!(parse_instance("x").is_err());
    }
}
