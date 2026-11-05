//! Configs, file secrets and gateway files on Docker Swarm, as native Swarm
//! objects instead of bind mounts of the deploy directory, so every node gets them
//! through Raft.
//!
//! Swarm objects are immutable, so each one is named after its content: the
//! deploy script hashes the file it is about to upload and creates
//! `<name>_<12 hex digits of its sha256>` unless that object exists already. The
//! same content keeps the same name and nothing changes. New content is a new
//! object, which changes the services that mount it, and Swarm rolls exactly those.
//! The hash is taken on the deploy target, from the real file, because a secret
//! with an `aws` source only has a value there.
//!
//! The stack files declare every object `external: true` with its name taken from
//! a variable the deploy script exports, and `docker stack deploy` interpolates it.
//! Creating and pruning them is done by `simpled step` (`deploy_steps`), from the
//! objects listed in the plan.

use crate::resolved_spec::{DeploymentResolvedSpec, ServiceResolvedSpec};
use crate::spec::SecretMount;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Swarm's limit for one config or secret.
pub const MAX_OBJECT_BYTES: usize = 500 * 1024;
/// Swarm names are at most 64 characters; `_` and 12 hex digits follow the base.
const MAX_BASE_LEN: usize = 64 - 13;
/// Read-only for everyone, so a service that does not run as root still reads it.
const MODE: u32 = 0o444;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Secret,
    Config,
}

impl ObjectKind {
    /// The `docker <kind>` command.
    pub fn command(self) -> &'static str {
        match self {
            ObjectKind::Secret => "secret",
            ObjectKind::Config => "config",
        }
    }
}

/// One object the deploy script syncs.
#[derive(Debug, Clone)]
pub struct SwarmObject {
    pub kind: ObjectKind,
    /// The name before the content hash, also the key the stack files use.
    pub base: String,
    /// Path of the file to upload, relative to the deploy directory.
    pub file: String,
    /// The variable the deploy script exports the object's full name in.
    pub var: String,
}

/// Where a service mounts an object. `source` is the stack file's key for it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ObjectMount {
    pub source: String,
    pub target: String,
    pub mode: u32,
    /// For a job, which is created with `docker service create` rather than from
    /// the stack file and names the object through the variable directly.
    #[serde(skip)]
    pub var: String,
}

/// A top-level `secrets:` or `configs:` entry of a stack file.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ExternalObject {
    pub external: bool,
    pub name: String,
}

/// The objects of one stack, and who owns them, which is what pruning is
/// scoped to.
#[derive(Debug, Default)]
pub struct SwarmObjects {
    /// `simpled.deployment=<name>` or `simpled.scope=environment`.
    pub owner_label: String,
    objects: Vec<SwarmObject>,
}

impl SwarmObjects {
    pub fn new(owner_label: String) -> Self {
        SwarmObjects {
            owner_label,
            objects: Vec::new(),
        }
    }

    /// Registers the object with this base name, once, and returns it.
    pub fn add(&mut self, kind: ObjectKind, base: &str, file: &str, var_prefix: &str) -> SwarmObject {
        let base = object_base(base);
        if let Some(existing) = self.objects.iter().find(|o| o.kind == kind && o.base == base) {
            return existing.clone();
        }
        let object = SwarmObject {
            kind,
            var: format!("{}{}", var_prefix, self.objects.len() + 1),
            base,
            file: file.replace('\\', "/"),
        };
        self.objects.push(object.clone());
        object
    }

    /// The stack file's top-level entries for one kind.
    pub fn externals(&self, kind: ObjectKind) -> BTreeMap<String, ExternalObject> {
        self.objects
            .iter()
            .filter(|o| o.kind == kind)
            .map(|o| {
                (
                    o.base.clone(),
                    ExternalObject {
                        external: true,
                        name: format!("${{{}}}", o.var),
                    },
                )
            })
            .collect()
    }

    /// The objects as the plan lists them, for the deploy steps.
    pub fn plan_objects(&self) -> Vec<crate::deploy_plan::PlanObject> {
        self.objects.iter().map(Into::into).collect()
    }
}

/// Swarm accepts letters, digits, `-`, `_` and `.`, starting and ending with a
/// letter or digit. Anything else becomes `_`, and a base too long for the hash to
/// fit is cut.
pub(crate) fn object_base(name: &str) -> String {
    let mut base: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    base.truncate(MAX_BASE_LEN);
    let trimmed = base.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        "object".to_string()
    } else {
        trimmed.to_string()
    }
}

/// The objects a deployment's services mount, and each service's mounts.
pub struct DeploymentObjects {
    pub objects: SwarmObjects,
    /// Keyed by service name: (secrets, configs).
    pub mounts: BTreeMap<String, (Vec<ObjectMount>, Vec<ObjectMount>)>,
    /// Literal values and config files to write under the deployment directory,
    /// as (path relative to the deploy directory, content).
    pub files: Vec<(String, Vec<u8>)>,
}

/// Where the upload files of a deployment's objects live, relative to the deploy
/// directory.
pub fn secret_file(deployment: &str, secret: &str) -> String {
    format!("{}/secrets/{}", deployment, secret)
}

fn config_file(deployment: &str, config: &str, file: &str) -> String {
    format!("{}/configs/{}/{}", deployment, config, file)
}

/// Every config file and file-mounted secret of the deployment's services. A
/// config mounted as a directory becomes one object per file, each mounted at
/// `<mount path>/<file>`. A secret whose value is only fetched on the target is
/// written by `fetch-secrets.sh` to `secret_file`, before the objects are synced.
pub fn collect(deployment: &DeploymentResolvedSpec) -> Result<DeploymentObjects> {
    let mut objects = SwarmObjects::new(format!("simpled.deployment={}", deployment.name));
    let mut mounts = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();

    for service in &deployment.services {
        let (secrets, configs) = service_mounts(deployment, service, &mut objects, &mut files)?;
        mounts.insert(service.full_name.clone(), (secrets, configs));
    }

    Ok(DeploymentObjects {
        objects,
        mounts,
        files: files.into_iter().collect(),
    })
}

fn service_mounts(
    deployment: &DeploymentResolvedSpec,
    service: &ServiceResolvedSpec,
    objects: &mut SwarmObjects,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<ObjectMount>, Vec<ObjectMount>)> {
    let var_prefix = "SIMPLED_OBJECT_";
    let mut secrets = Vec::new();
    for option in &service.secrets {
        let SecretMount::FilePath(target) = &option.mount else {
            continue;
        };
        let secret = deployment
            .secrets
            .iter()
            .find(|s| s.name == option.name)
            .ok_or_else(|| anyhow!("Service {} mounts unknown secret {}", service.full_name, option.name))?;
        let file = secret_file(&deployment.name, &secret.name);
        if let Some(value) = secret.literal() {
            check_size(value.len(), || format!("Secret '{}'", secret.name))?;
            files.insert(file.clone(), value.as_bytes().to_vec());
        }
        let object = objects.add(
            ObjectKind::Secret,
            &format!("{}_{}", deployment.name, secret.name),
            &file,
            var_prefix,
        );
        secrets.push(mount(&object, target));
    }

    let mut configs = Vec::new();
    for option in &service.configs {
        let config = deployment
            .configs
            .iter()
            .find(|c| c.name == option.config_name)
            .ok_or_else(|| {
                anyhow!(
                    "Service {} mounts unknown config {}",
                    service.full_name,
                    option.config_name
                )
            })?;
        // The same rule as the bind mounts: one file whose name the mount path
        // ends in is mounted as that file, anything else as a directory.
        let mounted_as_file = config.files.len() == 1
            && std::path::Path::new(&option.mount_path)
                .file_name()
                .is_some_and(|name| name.to_string_lossy() == config.files[0].name);
        for entry in &config.files {
            check_size(entry.content.len(), || {
                format!("File '{}' of config '{}'", entry.name, config.name)
            })?;
            let file = config_file(&deployment.name, &config.name, &entry.name);
            files.insert(file.clone(), entry.content.clone());
            let object = objects.add(
                ObjectKind::Config,
                &format!("{}_{}_{}", deployment.name, config.name, entry.name),
                &file,
                var_prefix,
            );
            let target = if mounted_as_file {
                option.mount_path.clone()
            } else {
                format!("{}/{}", option.mount_path.trim_end_matches('/'), entry.name)
            };
            configs.push(mount(&object, &target));
        }
    }
    Ok((secrets, configs))
}

fn mount(object: &SwarmObject, target: &str) -> ObjectMount {
    ObjectMount {
        source: object.base.clone(),
        target: target.to_string(),
        mode: MODE,
        var: object.var.clone(),
    }
}

fn check_size(len: usize, what: impl Fn() -> String) -> Result<()> {
    if len > MAX_OBJECT_BYTES {
        return Err(anyhow!(
            "{} is {} KB, but a Docker Swarm config or secret holds at most {} KB. Keep large files in the image, or in a volume bound under deployments.<name>.volumes",
            what(),
            len.div_ceil(1024),
            MAX_OBJECT_BYTES / 1024
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_base_name_is_made_safe_and_leaves_room_for_the_hash() {
        assert_eq!(
            object_base("prod_shop-data_settings.json"),
            "prod_shop-data_settings.json"
        );
        assert_eq!(object_base("prod_my app/x"), "prod_my_app_x");
        let long = object_base(&"a".repeat(80));
        assert_eq!(long.len(), MAX_BASE_LEN);
        assert_eq!(object_base("_x_"), "x");
    }

    #[test]
    fn an_object_is_registered_once_and_named_by_a_variable() {
        let mut objects = SwarmObjects::new("simpled.deployment=prod".to_string());
        let a = objects.add(ObjectKind::Config, "prod_cfg", "prod/configs/cfg/a", "V_");
        let again = objects.add(ObjectKind::Config, "prod_cfg", "prod/configs/cfg/a", "V_");
        let b = objects.add(ObjectKind::Secret, "prod_key", "prod/secrets/key", "V_");
        assert_eq!(a.var, again.var);
        assert_eq!((a.var.as_str(), b.var.as_str()), ("V_1", "V_2"));
        assert_eq!(objects.plan_objects().len(), 2);
        assert_eq!(objects.plan_objects()[1].file, "prod/secrets/key");
        assert_eq!(objects.externals(ObjectKind::Secret)["prod_key"].name, "${V_2}");
    }

    #[test]
    fn an_object_over_the_swarm_limit_fails() {
        assert!(check_size(MAX_OBJECT_BYTES, || "x".to_string()).is_ok());
        let err = check_size(MAX_OBJECT_BYTES + 1, || "File 'big.bin' of config 'data'".to_string())
            .unwrap_err()
            .to_string();
        assert!(err.contains("File 'big.bin' of config 'data' is 501 KB"), "{}", err);
    }
}
