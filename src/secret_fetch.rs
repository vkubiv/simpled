//! Secrets that are declared with an `aws:` source are deliberately *not* read
//! while a deployment is prepared. Preparation usually happens in CI, and the
//! directory it produces is copied to the deploy target, so a value resolved
//! there would travel through the build artifact, the CI logs and whatever
//! transport is used in between.
//!
//! Instead the generators emit `fetch-secrets.sh`, a script that performs the
//! lookup with the AWS CLI on the machine that runs the deploy. This module
//! builds that script, and — for local runs, where the deploy target *is* this
//! machine — performs the same lookup directly.

use crate::resolved_spec::SecretResolvedSpec;
use crate::spec::AwsSecretRef;
use anyhow::{anyhow, Context, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// File name every generator uses for the script, so the deploy instructions are
/// the same whichever target the environment spec selects.
pub const SCRIPT_NAME: &str = "fetch-secrets.sh";

/// Wrap a value in single quotes so the shell passes it through verbatim.
pub fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The shell pipeline that prints one secret's value to stdout.
fn fetch_pipeline(reference: &AwsSecretRef) -> String {
    let mut pipeline = format!(
        "aws secretsmanager get-secret-value --secret-id {} --query SecretString --output text",
        sh_quote(&reference.secret_id)
    );
    if let Some(filter) = &reference.jq {
        pipeline.push_str(&format!(" | jq -r {}", sh_quote(filter)));
    }
    pipeline
}

/// Accumulates the body of `fetch-secrets.sh`. Each generator fetches the
/// deferred secrets it has and then appends the target-specific part: writing
/// secret files for Docker, `kubectl create secret` for Kubernetes.
pub struct FetchScript {
    lines: Vec<String>,
    /// Every path the script creates on the deploy target, parent directories
    /// included, so that none of them is handed back to the invoking user twice.
    created: Vec<String>,
    /// Whether anything is handed back, i.e. whether the script needs the
    /// `simpled_give_back` helper defined in its preamble.
    gives_back: bool,
    needs_jq: bool,
    fetched: bool,
}

/// Name of the helper the script uses to hand what it creates back to the user
/// that invoked sudo. Prefixed like the other shell state this script leaves
/// behind, because `deploy.sh` sources it.
const GIVE_BACK_FUNCTION_NAME: &str = "simpled_give_back";

impl FetchScript {
    pub fn new() -> Self {
        FetchScript {
            lines: Vec::new(),
            created: Vec::new(),
            gives_back: false,
            needs_jq: false,
            fetched: false,
        }
    }

    /// Whether anything has been fetched yet — generators skip writing the script
    /// (and calling it) when the deployment has no deferred secrets.
    pub fn is_empty(&self) -> bool {
        !self.fetched
    }

    /// Look up one secret and export it as its `shell_var()`.
    pub fn fetch(&mut self, secret: &SecretResolvedSpec, reference: &AwsSecretRef) {
        self.fetched = true;
        self.needs_jq |= reference.jq.is_some();
        let var = secret.shell_var();
        self.lines.push(format!("echo 'Fetching secret {} from AWS Secrets Manager...'", secret.name));
        self.lines.push(format!("{}=\"$({})\"", var, fetch_pipeline(reference)));
        // An empty value means the lookup silently produced nothing (a missing jq
        // field, most often) and would deploy a service with a blank credential.
        self.lines.push(format!(
            "if [ -z \"${{{var}}}\" ]; then echo 'Secret {name} resolved to an empty value.' >&2; exit 1; fi",
            var = var,
            name = secret.name
        ));
        self.lines.push(format!("export {}", var));
    }

    /// Write an already-fetched secret to `path`, relative to the directory the
    /// script runs in.
    pub fn write_to_file(&mut self, secret: &SecretResolvedSpec, path: &str) {
        let path = path.replace('\\', "/");
        let mut new_paths = Vec::new();
        if let Some(parent) = path.rsplit_once('/').map(|(parent, _)| parent) {
            self.lines.push(format!("mkdir -p {}", sh_quote(parent)));
            // `mkdir -p` creates the whole chain, and any part of it may be the one
            // that did not exist yet, so every ancestor is handed back too.
            let mut prefix = String::new();
            for component in parent.split('/').filter(|c| !c.is_empty() && *c != ".") {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(component);
                new_paths.extend(self.record_created(prefix.clone()));
            }
        }
        self.lines.push(format!(
            "printf '%s' \"${}\" > {}",
            secret.shell_var(),
            sh_quote(&path)
        ));
        new_paths.extend(self.record_created(path));

        // Right here rather than at the end of the script: a secret that fails to
        // resolve aborts the deploy, and what was written before it must not be
        // left behind owned by root either.
        if !new_paths.is_empty() {
            self.gives_back = true;
            self.lines.push(format!(
                "{} {}",
                GIVE_BACK_FUNCTION_NAME,
                new_paths.iter().map(|path| sh_quote(path)).collect::<Vec<_>>().join(" ")
            ));
        }
    }

    /// Remember `path` as created by the script, and report whether it is the
    /// first time — a path handed back once needs no second `chown`.
    fn record_created(&mut self, path: String) -> Option<String> {
        if self.created.contains(&path) {
            return None;
        }
        self.created.push(path.clone());
        Some(path)
    }

    /// Append a raw command to the target-specific part of the script.
    pub fn command(&mut self, line: String) {
        self.lines.push(line);
    }

    /// Write the script out. `usage` is the one-line instruction shown in the
    /// header, which differs per target: sourced by `deploy.sh` for Docker, run by
    /// hand before `kubectl apply` for Kubernetes.
    pub fn write(&self, path: &Path, usage: &str) -> Result<()> {
        let mut file = File::create(path)
            .with_context(|| format!("Failed to create {:?}", path))?;

        #[cfg(unix)]
        {
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(0o755);
            file.set_permissions(perms)?;
        }

        writeln!(file, "#!/bin/bash")?;
        writeln!(file, "# Generated by simpled. Reads the secrets declared with an `aws:` source from")?;
        writeln!(file, "# AWS Secrets Manager, here on the deploy target, so their values never travel")?;
        writeln!(file, "# inside the generated deployment directory.")?;
        writeln!(file, "#")?;
        writeln!(file, "# {}", usage)?;
        writeln!(file, "#")?;
        writeln!(file, "# Credentials, region and profile are taken from the AWS CLI's own environment")?;
        writeln!(file, "# (instance role, AWS_REGION, AWS_PROFILE, ...).")?;
        writeln!(file, "set -e")?;
        writeln!(file)?;
        writeln!(file, "command -v aws >/dev/null 2>&1 || {{ echo 'The AWS CLI is required to fetch secrets but was not found on PATH.' >&2; exit 1; }}")?;
        if self.needs_jq {
            writeln!(file, "command -v jq >/dev/null 2>&1 || {{ echo 'jq is required by the jq filter of at least one secret but was not found on PATH.' >&2; exit 1; }}")?;
        }
        writeln!(file)?;
        // Restored afterwards because this script is sourced: leaving a tightened
        // umask behind would silently change the permissions of everything the
        // calling deploy script creates after it.
        writeln!(file, "# Keep the files this script writes readable by their owner only.")?;
        writeln!(file, "simpled_previous_umask=\"$(umask)\"")?;
        writeln!(file, "umask 077")?;
        writeln!(file)?;

        // A deploy script needs root for Docker, so it is run with sudo. Everything
        // this script writes would then be owned by root, and the unprivileged user
        // that copies the next deployment onto the target could no longer overwrite
        // it — the second deploy would fail on a permission error, on the secret
        // files of all things. Ownership is handed back; the permissions the umask
        // above sets are left alone, so the values stay as readable as they were.
        if self.gives_back {
            writeln!(file, "# Run with sudo, everything below would belong to root, and the user that")?;
            writeln!(file, "# copies the next deployment in could no longer overwrite it. Give each path")?;
            writeln!(file, "# back to the user that invoked sudo, permissions untouched.")?;
            writeln!(file, "{}() {{", GIVE_BACK_FUNCTION_NAME)?;
            writeln!(file, "  [ \"$(id -u)\" = '0' ] || return 0")?;
            writeln!(file, "  [ -n \"${{SUDO_UID:-}}\" ] || return 0")?;
            writeln!(file, "  chown \"$SUDO_UID:${{SUDO_GID:-$SUDO_UID}}\" \"$@\" \\")?;
            writeln!(file, "    || echo \"Warning: $* stay owned by root.\" >&2")?;
            writeln!(file, "}}")?;
            writeln!(file)?;
        }

        for line in &self.lines {
            writeln!(file, "{}", line)?;
        }

        writeln!(file)?;
        writeln!(file, "umask \"$simpled_previous_umask\"")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::SecretResolvedValue;

    fn secret(name: &str) -> SecretResolvedSpec {
        SecretResolvedSpec {
            name: name.to_string(),
            value: SecretResolvedValue::Literal(String::new()),
        }
    }

    fn script(reference: AwsSecretRef) -> String {
        let mut script = FetchScript::new();
        let secret = secret("shop-db_password");
        script.fetch(&secret, &reference);
        script.write_to_file(&secret, "secrets/shop-db_password");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SCRIPT_NAME);
        script.write(&path, "usage").unwrap();
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn a_plain_secret_needs_only_the_aws_cli() {
        let out = script(AwsSecretRef { secret_id: "prod/shop/db".to_string(), jq: None });

        assert!(out.contains("command -v aws"));
        assert!(!out.contains("command -v jq"));
        assert!(out.contains(
            "SIMPLED_SECRET_SHOP_DB_PASSWORD=\"$(aws secretsmanager get-secret-value \
             --secret-id 'prod/shop/db' --query SecretString --output text)\""
        ));
        assert!(out.contains("mkdir -p 'secrets'"));
        // Run with sudo, the script gives what it wrote back to the user that
        // invoked sudo — the directory included — so the next deployment can still
        // be copied over it.
        assert!(out.contains("chown \"$SUDO_UID:${SUDO_GID:-$SUDO_UID}\" \"$@\""));
        assert!(out.contains("simpled_give_back 'secrets' 'secrets/shop-db_password'"));
        // Sourced by the deploy script, so the umask it tightens is put back.
        assert!(out.contains("umask \"$simpled_previous_umask\""));
    }

    /// Single quotes in a secret id or filter must not be able to break out of the
    /// quoting and become shell syntax in the generated script.
    #[test]
    fn quotes_are_escaped() {
        let out = script(AwsSecretRef {
            secret_id: "prod/'; rm -rf /; '".to_string(),
            jq: Some(".pass'word".to_string()),
        });

        assert!(out.contains("command -v jq"));
        assert!(out.contains("--secret-id 'prod/'\\''; rm -rf /; '\\'''"));
        assert!(out.contains("jq -r '.pass'\\''word'"));
    }
}

/// Perform the lookup on this machine. Used for `local` deployments, where the
/// deploy target is the machine running simpled and there is no deploy script to
/// defer the work to.
pub fn fetch_locally(reference: &AwsSecretRef) -> Result<String> {
    let output = Command::new("aws")
        .args([
            "secretsmanager",
            "get-secret-value",
            "--secret-id",
            &reference.secret_id,
            "--query",
            "SecretString",
            "--output",
            "text",
        ])
        .output()
        .context("Failed to run the AWS CLI. It is required to read secrets with an 'aws' source.")?;

    if !output.status.success() {
        return Err(anyhow!(
            "aws secretsmanager get-secret-value failed for '{}': {}",
            reference.secret_id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let value = String::from_utf8(output.stdout)
        .context("AWS Secrets Manager returned a value that is not valid UTF-8")?;
    // `--output text` terminates the value with a newline that is not part of it.
    let value = value.trim_end_matches(['\n', '\r']).to_string();

    let Some(filter) = &reference.jq else {
        return Ok(value);
    };

    let mut jq = Command::new("jq")
        .args(["-r", filter])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run jq. It is required by the jq filter of a secret with an 'aws' source.")?;

    jq.stdin
        .take()
        .expect("jq stdin was piped")
        .write_all(value.as_bytes())
        .context("Failed to pass the secret value to jq")?;

    let output = jq.wait_with_output().context("Failed to read the output of jq")?;
    if !output.status.success() {
        return Err(anyhow!(
            "jq filter '{}' failed for secret '{}': {}",
            filter,
            reference.secret_id,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let filtered = String::from_utf8(output.stdout)
        .context("jq returned a value that is not valid UTF-8")?;
    Ok(filtered.trim_end_matches(['\n', '\r']).to_string())
}

