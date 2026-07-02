use std::collections::HashMap;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct AppSpec {
    pub name: String,
    pub version: semver::Version,
    pub environment: AppEnvironment,    
    pub app_services: Vec<ServiceSpec>,
    pub extra_services: Vec<ServiceSpec>,
    pub configs: Vec<ConfigSpec>,
    pub secrets: Vec<AppSecretOption>,
    pub volumes: Vec<String>,
}

impl AppSpec {
    pub fn all_services(&self) -> impl Iterator<Item = &ServiceSpec> {
        self.app_services.iter().chain(self.extra_services.iter())
    }
}

#[derive(Debug, Clone)]
pub struct ConfigSpec {
    pub name: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalEnvVariable {
    pub name: String,
    pub default: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OptionalEnvVariable {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct RelativeEnvVariable {
    pub name: String,
    pub relative_value: String,
}

#[derive(Debug, Clone)]
pub struct InternalEnvVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct AppEnvironment {
    pub external: Vec<ExternalEnvVariable>,
    pub optional: Vec<OptionalEnvVariable>,
    pub relative: Vec<RelativeEnvVariable>,
    pub internal: Vec<InternalEnvVariable>,
}

#[derive(Debug, Clone)]
pub struct AppSecretOption {
    pub secret_name: String,
}

#[derive(Debug, Clone)]
pub struct ServicePort {
    pub external: u16,
    pub internal: u16,
}

#[derive(Debug, Clone)]
pub enum ServiceVolumeType {
    Named(String),
    Path(String),
}

#[derive(Debug, Clone)]
pub struct ServiceVolume {
    pub name: ServiceVolumeType,
    pub mount_path: String,
}

#[derive(Debug, Clone)]
pub struct ServiceSpec {
    pub name: String,
    pub service_type: ServiceType,
    pub is_app_service: bool,
    pub image: ImageSpec,
    pub environment: Vec<ServiceEnvOption>,
    pub configs: Vec<ServiceConfigOption>,
    pub secrets: Vec<ServiceSecret>,
    pub ports: Vec<ServicePort>,
    pub volumes: Vec<ServiceVolume>,
    // Overrides the image's default command, same as docker-compose `command`.
    pub command: Option<ServiceCommand>,
    // Overrides the image's ENTRYPOINT, same as docker-compose `entrypoint`.
    pub entrypoint: Option<ServiceCommand>,
    // Container health probe, same as docker-compose `healthcheck`.
    pub healthcheck: Option<Healthcheck>,
}

// Overrides the default command/entrypoint of a service's image. Mirrors
// docker-compose `command`/`entrypoint`, which accept either a shell string or
// an exec-form list of args.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ServiceCommand {
    Shell(String),
    Exec(Vec<String>),
}

impl ServiceCommand {
    /// Normalize to an argv vector. The shell (string) form is split on whitespace.
    pub fn to_args(&self) -> Vec<String> {
        match self {
            ServiceCommand::Shell(s) => s.split_whitespace().map(str::to_string).collect(),
            ServiceCommand::Exec(v) => v.clone(),
        }
    }
}

// Container health check. Mirrors docker-compose `healthcheck`. Durations use
// the compose format (e.g. "30s", "1m30s") and are serialized back unchanged for
// compose/swarm; they are parsed to whole seconds for Kubernetes probes.
#[derive(Debug, Clone, Serialize)]
pub struct Healthcheck {
    pub test: HealthcheckTest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_period: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub disable: bool,
}

// The `test` of a healthcheck: either a shell string (run via the container's
// shell) or an exec-form list whose first element is `CMD`, `CMD-SHELL` or `NONE`.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum HealthcheckTest {
    Shell(String),
    Exec(Vec<String>),
}

impl Healthcheck {
    /// A healthcheck is off when explicitly disabled or when `test` is `["NONE"]`.
    pub fn is_disabled(&self) -> bool {
        self.disable || matches!(&self.test, HealthcheckTest::Exec(v) if v.first().map(String::as_str) == Some("NONE"))
    }

    /// Exec-form argv for a Kubernetes exec probe, or `None` when disabled.
    /// A shell string / `CMD-SHELL` is wrapped in `/bin/sh -c`; `CMD` drops the
    /// keyword and runs the remaining args directly.
    pub fn probe_argv(&self) -> Option<Vec<String>> {
        if self.is_disabled() {
            return None;
        }
        match &self.test {
            HealthcheckTest::Shell(s) => Some(vec!["/bin/sh".into(), "-c".into(), s.clone()]),
            HealthcheckTest::Exec(v) => match v.first().map(String::as_str) {
                Some("CMD") => Some(v[1..].to_vec()),
                Some("CMD-SHELL") => Some(vec!["/bin/sh".into(), "-c".into(), v[1..].join(" ")]),
                _ => Some(v.clone()),
            },
        }
    }

    /// Shell command string for a `docker run --health-cmd`, or `None` when
    /// disabled. Exec-form `CMD`/`CMD-SHELL` args are joined into one string.
    pub fn health_cmd_string(&self) -> Option<String> {
        if self.is_disabled() {
            return None;
        }
        match &self.test {
            HealthcheckTest::Shell(s) => Some(s.clone()),
            HealthcheckTest::Exec(v) => match v.first().map(String::as_str) {
                Some("CMD") | Some("CMD-SHELL") => Some(v[1..].join(" ")),
                _ => Some(v.join(" ")),
            },
        }
    }
}

/// Parse a compose duration (e.g. "30s", "1m30s", "1h") into whole seconds.
/// Supports `h`, `m`, `s`, `ms` and `us`/`µs` segments; sub-second parts round up.
pub fn parse_duration_secs(input: &str) -> Option<u64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    let mut total_ms: u128 = 0;
    let mut num = String::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            num.push(c);
            chars.next();
        } else {
            let value: u128 = num.parse().ok()?;
            num.clear();
            // Read the (possibly multi-char) unit.
            let mut unit = String::new();
            while let Some(&u) = chars.peek() {
                if u.is_ascii_digit() {
                    break;
                }
                unit.push(u);
                chars.next();
            }
            let factor_ms = match unit.as_str() {
                "h" => 3_600_000,
                "m" => 60_000,
                "s" => 1_000,
                "ms" => 1,
                "us" | "µs" => 0, // sub-millisecond: ignore for whole-second probes
                _ => return None,
            };
            total_ms += value * factor_ms;
        }
    }
    // A trailing bare number (no unit) is invalid in the compose format.
    if !num.is_empty() {
        return None;
    }
    // Round up to the next whole second so a non-zero duration never becomes 0.
    Some(((total_ms + 999) / 1000) as u64)
}

#[derive(Debug, Clone)]
pub enum ServiceEnvOption {
    All,
    Simple(String),
    WithValue(String, String),
}

#[derive(Debug, Clone)]
pub struct ServiceConfigOption {
    pub config_name: String,
    pub mount_path: String,
}


#[derive(Debug, Clone)]
pub struct ImageVariant {
    pub variant_name: String,
    pub image: String,
}

#[derive(Debug, Clone)]
pub enum ImageSpec {
    Exact(String),
    Variants(Vec<ImageVariant>),
}

#[derive(Debug, Clone)]
pub enum ServiceType {
    Public,
    Internal,
    Job,
}

#[derive(Debug, Clone)]
pub struct  ServiceSecret {
    pub name: String,
    pub mount: SecretMount,
}

#[derive(Debug, Clone)]
pub enum SecretMount {
    FilePath(String),
    EnvVariable(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeploymentEnvType {
    K8S,
    Docker(DockerSpecificSpec),
    Local,
}

#[derive(Debug, Clone, PartialEq)]
pub enum  DockerIngressType {
    Nginx,
    Traefik,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DockerSpecificSpec {
    pub ingress_type: DockerIngressType,
    pub swarm_mode: bool,
}

// DeploymentEnvironmentSpec definitions
#[derive(Debug, Clone)]
pub struct DeploymentEnvironmentSpec {
    pub env_type: DeploymentEnvType,
    pub ingress: IngressSpec,
    pub registry: HashMap<String, String>,
    pub deployments: Vec<DeploymentSpec>,
}

#[derive(Debug, Clone)]
pub struct IngressSpec {
    pub name: String,
    pub hosts: Vec<HostSpec>,
    pub tls: Option<IngressTlsSpec>,
}

#[derive(Debug, Clone)]
pub struct IngressTlsSpec {
    pub secret: Option<String>,
    pub letsencrypt: Option<LetsEncryptSpec>,
}

#[derive(Debug, Clone)]
pub struct LetsEncryptSpec {
    pub server: Option<String>,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct HostSpec {
    pub name: String,
    pub domain_names: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct DeploymentSpec {
    pub name: String,
    pub primary_host: String,
    pub application: DeploymentAppSpec,
    pub environment: Vec<EnvVariable>,
    pub undockerized_environment: Vec<EnvVariable>,
    pub configs: Vec<ConfigSpec>,
    pub secrets: Vec<DeploymentSecretSpec>,
    pub defaults: ResourcesSpec,
    pub services: Option<HashMap<String, DeploymentServiceSpec>>,
}

#[derive(Debug, Clone)]
pub struct DeploymentSecretSpec {
    pub secret_name: String,
    pub source: DeploymentSecretSource,
}

#[derive(Debug, Clone)]
pub enum DeploymentSecretSource {
    EnvVariable(String),
    FilePath(String),
    Embedded(String),
}

#[derive(Debug, Clone)]
pub struct DeploymentAppSpec {
    pub name: String,
    pub version: Option<semver::VersionReq>,
    pub extra: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ResourcesSpec {
    pub replicas: u32,
    pub requests: ResourceLimits,
    pub limits: ResourceLimits,
}

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub memory: String,
    pub cpu: String,
}

#[derive(Debug, Clone)]
pub struct DeploymentServiceSpec {
    pub variant: Option<String>,
    pub host: Option<String>,
    pub prefixes: Vec<Prefix>,
    pub resources: ResourcesSpec,
    pub ports: Vec<ServicePort>,
    // local-only: working directory of a host-run (non-dockerized) service.
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Prefix {
    pub prefix: String,
    pub strip: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_durations() {
        assert_eq!(parse_duration_secs("30s"), Some(30));
        assert_eq!(parse_duration_secs("2m"), Some(120));
        assert_eq!(parse_duration_secs("1h"), Some(3600));
    }

    #[test]
    fn parses_compound_and_subsecond_durations() {
        assert_eq!(parse_duration_secs("1m30s"), Some(90));
        assert_eq!(parse_duration_secs("1h1m1s"), Some(3661));
        // Sub-second values round up so a non-zero duration never becomes 0.
        assert_eq!(parse_duration_secs("500ms"), Some(1));
        assert_eq!(parse_duration_secs("1s500ms"), Some(2));
    }

    #[test]
    fn rejects_invalid_durations() {
        assert_eq!(parse_duration_secs("30"), None); // no unit
        assert_eq!(parse_duration_secs("abc"), None);
        assert_eq!(parse_duration_secs(""), None);
    }

    #[test]
    fn healthcheck_probe_argv_maps_test_forms() {
        let shell = Healthcheck {
            test: HealthcheckTest::Shell("curl -f localhost".into()),
            interval: None, timeout: None, retries: None, start_period: None, disable: false,
        };
        assert_eq!(shell.probe_argv(), Some(vec!["/bin/sh".into(), "-c".into(), "curl -f localhost".into()]));

        let cmd = Healthcheck {
            test: HealthcheckTest::Exec(vec!["CMD".into(), "curl".into(), "-f".into(), "localhost".into()]),
            interval: None, timeout: None, retries: None, start_period: None, disable: false,
        };
        assert_eq!(cmd.probe_argv(), Some(vec!["curl".into(), "-f".into(), "localhost".into()]));

        let cmd_shell = Healthcheck {
            test: HealthcheckTest::Exec(vec!["CMD-SHELL".into(), "curl -f localhost".into()]),
            interval: None, timeout: None, retries: None, start_period: None, disable: false,
        };
        assert_eq!(cmd_shell.probe_argv(), Some(vec!["/bin/sh".into(), "-c".into(), "curl -f localhost".into()]));
    }

    #[test]
    fn disabled_healthcheck_has_no_probe() {
        let by_flag = Healthcheck {
            test: HealthcheckTest::Shell("x".into()),
            interval: None, timeout: None, retries: None, start_period: None, disable: true,
        };
        assert!(by_flag.is_disabled());
        assert_eq!(by_flag.probe_argv(), None);

        let by_none = Healthcheck {
            test: HealthcheckTest::Exec(vec!["NONE".into()]),
            interval: None, timeout: None, retries: None, start_period: None, disable: false,
        };
        assert!(by_none.is_disabled());
        assert_eq!(by_none.probe_argv(), None);
    }
}
