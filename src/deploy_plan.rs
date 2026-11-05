//! `plan.json`: what the deploy steps (`simpled step ...`) act on, written by
//! `prepare-deployment` next to the stack files. A step never reads the env or
//! app spec: those depend on the CI environment, on files next to the spec and
//! on the downloaded bundle, none of which exist on the deploy target. A plan is
//! the resolved result.
//!
//! Every string in a plan that ends up in a command is a template in compose's
//! interpolation syntax, the one `docker stack deploy` applies to stack files:
//! `${NAME}` is a variable from the step's environment and `$$` a literal `$`.
//! A literal secret value is therefore escaped once and reaches a stack service
//! and a job the same way.
//!
//! A plan holds no more than the stack file does. File secrets appear only as
//! paths, and secrets with an `aws` source only as the variables
//! `fetch-secrets.sh` exports. A literal secret mounted as an environment variable
//! is in a job's arguments with its value, as it is in the stack file's
//! `environment:` and in the service spec Docker keeps.

use crate::swarm_objects::{ObjectKind, SwarmObject};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// The plan of one owner: the gateway's (`ingress/plan.json`) or a deployment's
/// (`<deployment>/plan.json`).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Plan {
    /// The simpled that prepared the deployment, and shipped the binary that runs
    /// the steps.
    pub simpled_version: String,
    /// The label every object of this plan carries, which pruning is scoped to.
    pub owner_label: String,
    #[serde(default)]
    pub objects: Vec<PlanObject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<PlanJob>,
    /// The gateway's plan only: the domains it holds a certificate secret for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tls: Vec<PlanTls>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlanTls {
    pub domain: String,
    /// The variable the ingress stack reads the secret's name from.
    pub var: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlanObject {
    pub kind: ObjectKind,
    /// The name before the content hash.
    pub base: String,
    /// The file the object is created from, relative to the deploy directory.
    pub file: String,
    /// The variable its full name is exported in.
    pub var: String,
}

impl From<&SwarmObject> for PlanObject {
    fn from(object: &SwarmObject) -> Self {
        PlanObject {
            kind: object.kind,
            base: object.base.clone(),
            file: object.file.clone(),
            var: object.var.clone(),
        }
    }
}

/// A job, run to completion as a Swarm `replicated-job` service.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PlanJob {
    /// The name in the app spec, which `step run-job` is called with.
    pub name: String,
    /// The Swarm service it runs as, `<deployment>_<job>`.
    pub service: String,
    /// Everything `docker service create` takes after the flags every job gets,
    /// image and command included. Templates.
    pub args: Vec<String>,
    /// Seconds to wait for the job to finish, unless `JOB_TIMEOUT` overrides it.
    pub timeout_seconds: u32,
}

impl Plan {
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json + "\n").with_context(|| format!("Failed to write {}", path.display()))
    }

    pub fn read(path: &Path) -> Result<Plan> {
        let text = fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("{} is not a valid plan", path.display()))
    }

    pub fn job(&self, name: &str) -> Result<&PlanJob> {
        self.jobs.iter().find(|j| j.name == name).ok_or_else(|| {
            let known: Vec<&str> = self.jobs.iter().map(|j| j.name.as_str()).collect();
            anyhow!(
                "The plan has no job '{}'. Jobs: {}",
                name,
                if known.is_empty() {
                    "none".to_string()
                } else {
                    known.join(", ")
                }
            )
        })
    }
}

/// A literal, written into a template: every `$` doubled.
pub fn escape(literal: &str) -> String {
    literal.replace('$', "$$")
}

/// Expands a template the way compose does: `$$` is `$`, `${NAME}` is the value
/// `lookup` gives for `NAME`, and a variable `lookup` does not know is an error
/// rather than an empty string, since an empty object name or constraint would
/// only fail later and less clearly.
pub fn interpolate(template: &str, lookup: impl Fn(&str) -> Option<String>) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find('$') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(after) = rest.strip_prefix('$') {
            out.push('$');
            rest = after;
        } else if let Some(after) = rest.strip_prefix('{') {
            let end = after
                .find('}')
                .ok_or_else(|| anyhow!("Unclosed ${{ in '{}'", template))?;
            let name = &after[..end];
            let value = lookup(name).ok_or_else(|| {
                anyhow!(
                    "{} is not set. deploy.sh exports it before this step; run the step from deploy.sh",
                    name
                )
            })?;
            out.push_str(&value);
            rest = &after[end + 1..];
        } else {
            out.push('$');
        }
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(name: &str) -> Option<String> {
        match name {
            "OBJ" => Some("prod_key_3f9a".to_string()),
            "EMPTY" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn templates_expand_like_compose() {
        assert_eq!(
            interpolate("source=${OBJ},target=/k", env).unwrap(),
            "source=prod_key_3f9a,target=/k"
        );
        assert_eq!(interpolate("pa$$word$$$$x", env).unwrap(), "pa$word$$x");
        assert_eq!(interpolate("a=${EMPTY}.", env).unwrap(), "a=.");
        assert_eq!(interpolate("cost $5", env).unwrap(), "cost $5");
        let err = interpolate("${MISSING}", env).unwrap_err().to_string();
        assert!(err.starts_with("MISSING is not set"), "{}", err);
        assert!(interpolate("${OBJ", env).is_err());
    }

    #[test]
    fn an_escaped_literal_comes_back_unchanged() {
        for literal in ["pa$word", "${NOT_A_VAR}", "$$", "plain"] {
            assert_eq!(interpolate(&escape(literal), env).unwrap(), literal);
        }
    }

    #[test]
    fn a_plan_survives_a_round_trip() {
        let plan = Plan {
            simpled_version: "2.0.0".to_string(),
            owner_label: "simpled.deployment=prod".to_string(),
            objects: vec![PlanObject {
                kind: ObjectKind::Secret,
                base: "prod_key".to_string(),
                file: "prod/secrets/key".to_string(),
                var: "SIMPLED_OBJECT_1".to_string(),
            }],
            jobs: vec![PlanJob {
                name: "migrate".to_string(),
                service: "prod_migrate".to_string(),
                args: vec!["--network".to_string(), "common_network".to_string()],
                timeout_seconds: 600,
            }],
            tls: vec![PlanTls {
                domain: "shop.example.com".to_string(),
                var: "SIMPLED_TLS_1".to_string(),
            }],
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prod").join("plan.json");
        plan.write(&path).unwrap();
        assert_eq!(Plan::read(&path).unwrap(), plan);
        assert!(fs::read_to_string(&path).unwrap().contains("\"kind\": \"secret\""));
        assert!(plan.job("seed").unwrap_err().to_string().contains("Jobs: migrate"));
    }
}
