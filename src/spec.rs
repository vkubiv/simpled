use serde::Serialize;
use std::collections::HashMap;

/// Renders a version for the places that cannot carry a `+`.
///
/// A side-branch version keeps its label as semver build metadata
/// (`1.0.2+big-refactor`), but `+` is not legal in a docker tag and decodes to a space
/// in a URL query parameter, so docker tags, bundle file names and release tags use `-`
/// instead: `1.0.2-big-refactor`. Versions without build metadata are unchanged.
pub fn version_to_tag(version: &str) -> String {
    version.replace('+', "-")
}

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
    // Internal-only ports (not published to the host), same as docker-compose `expose`.
    pub expose: Vec<String>,
    pub volumes: Vec<ServiceVolume>,
    // Overrides the image's default command, same as docker-compose `command`.
    pub command: Option<ServiceCommand>,
    // Overrides the image's ENTRYPOINT, same as docker-compose `entrypoint`.
    pub entrypoint: Option<ServiceCommand>,
    // Container health probe, same as docker-compose `healthcheck`.
    pub healthcheck: Option<Healthcheck>,
    // Names of services that must be started before this one, same as
    // docker-compose `depends_on`. `None` when the spec does not mention it,
    // which for a job means "every long-running service"; `Some(vec![])` is an
    // explicit "nothing in the stack" (the job's dependencies are external).
    pub depends_on: Option<Vec<String>>,
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

/// Parse a request body size (e.g. "10m", "512k", "1G", "2mb", "1048576") into
/// bytes, in the notation nginx's `client_max_body_size` uses.
///
/// `0` is not "no size" but nginx's spelling of "no limit", and is returned as
/// such; the generators pass it through rather than treating it as unset.
pub fn parse_body_size_bytes(input: &str) -> Option<u64> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let value: u64 = digits.parse().ok()?;

    // An optional `b` is accepted so that "10mb" reads the same as "10m".
    let unit = s[digits.len()..].trim().to_ascii_lowercase();
    let factor: u64 = match unit.trim_end_matches('b') {
        "" => 1,
        "k" => 1024,
        "m" => 1024 * 1024,
        "g" => 1024 * 1024 * 1024,
        _ => return None,
    };
    // "10b" is bytes, but a bare "b" after nothing ("10bb") is not a unit.
    if unit.matches('b').count() > 1 {
        return None;
    }

    value.checked_mul(factor)
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
    Some(total_ms.div_ceil(1000) as u64)
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
pub struct ServiceSecret {
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
pub struct DockerSpecificSpec {
    pub swarm_mode: bool,
    /// Swarm only: validate as if the swarm had several nodes even when it has
    /// one. When false, deploy.sh refuses a swarm with more than one schedulable
    /// node and pins every service to the node it runs on. Always false outside
    /// Swarm.
    pub multi_node: bool,
    /// Swarm only: the OS and CPU of the deploy target, which the shipped simpled
    /// binary has to run on.
    pub target_platform: TargetPlatform,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TargetPlatform {
    #[default]
    LinuxAmd64,
    LinuxArm64,
}

impl TargetPlatform {
    /// The architecture in release asset names.
    pub fn arch(self) -> &'static str {
        match self {
            TargetPlatform::LinuxAmd64 => "amd64",
            TargetPlatform::LinuxArm64 => "arm64",
        }
    }

    /// The architecture as `std::env::consts::ARCH` names it.
    pub fn rust_arch(self) -> &'static str {
        match self {
            TargetPlatform::LinuxAmd64 => "x86_64",
            TargetPlatform::LinuxArm64 => "aarch64",
        }
    }
}

impl std::fmt::Display for TargetPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "linux-{}", self.arch())
    }
}

/// Where a named volume lives in a Swarm environment.
#[derive(Debug, Clone, PartialEq)]
pub enum VolumeBinding {
    /// A Docker volume (local driver) on one node.
    Node { node: String },
    /// A host directory on one node.
    NodePath { node: String, path: String },
    /// A host path that exists on every node, such as a shared mount.
    Path { path: String },
    /// A Docker volume backed by NFS, mounted by whichever node runs the task.
    Nfs(NfsVolume),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NfsVolume {
    pub address: String,
    pub path: String,
    /// Extra NFS mount options, appended after `addr=<address>`.
    pub options: Option<String>,
}

impl VolumeBinding {
    /// The node every service mounting this volume has to run on.
    pub fn node(&self) -> Option<&str> {
        match self {
            VolumeBinding::Node { node } | VolumeBinding::NodePath { node, .. } => Some(node),
            _ => None,
        }
    }
}

impl NfsVolume {
    /// `o=` option of the local driver's NFS mount.
    pub fn mount_options(&self) -> String {
        match &self.options {
            Some(options) => format!("addr={},{}", self.address, options),
            None => format!("addr={}", self.address),
        }
    }
}

// DeploymentEnvironmentSpec definitions
#[derive(Debug, Clone)]
pub struct DeploymentEnvironmentSpec {
    pub env_type: DeploymentEnvType,
    pub ingress: IngressSpec,
    pub registry: HashMap<String, String>,
    pub deployments: Vec<DeploymentSpec>,
    pub ports: crate::ports::LocalPorts,
    /// A local deployment run from a released bundle: app services take the
    /// images CI published for its version, through `registry`, instead of
    /// `:latest` from the local Docker cache.
    pub published_images: bool,
}

impl DeploymentEnvironmentSpec {
    /// The deployment called `name`, or an error naming the ones that exist.
    pub fn deployment(&self, name: &str) -> anyhow::Result<&DeploymentSpec> {
        self.deployments.iter().find(|d| d.name == name).ok_or_else(|| {
            let mut available: Vec<&str> = self.deployments.iter().map(|d| d.name.as_str()).collect();
            available.sort();
            anyhow::anyhow!(
                "Deployment '{}' not found in env spec. Available deployments: {}",
                name,
                available.join(", ")
            )
        })
    }
}

#[derive(Debug, Clone)]
pub struct IngressSpec {
    pub name: String,
    pub hosts: Vec<HostSpec>,
    pub tls: Option<IngressTlsSpec>,
    pub redirects: Vec<RedirectSpec>,
    /// Default maximum request body, in bytes, for every route the gateway
    /// serves. `None` leaves the gateway's own default in place.
    pub body_limit: Option<u64>,
    /// IPs/CIDRs of the proxies in front of the gateway whose X-Forwarded-For
    /// is believed. Empty trusts no one, the gateway's own default.
    pub trusted_proxies: Vec<String>,
    /// Swarm only: one gateway task per node instead of a single replica.
    pub global: bool,
    /// Swarm only: where the gateway task(s) may run.
    pub placement: Option<Placement>,
    /// Docker only: the controller's private API.
    pub controller: Option<ControllerSpec>,
}

/// `controller`: where the controller's private API is routed, and its tokens.
#[derive(Debug, Clone)]
pub struct ControllerSpec {
    /// Host alias and the path prefix the API is served under, which the gateway
    /// strips.
    pub hosts: Vec<(String, String)>,
    pub read_token: Option<DeploymentSecretSpec>,
    pub manage_token: Option<DeploymentSecretSpec>,
}

/// A domain the gateway serves only to bounce the client to another one, such as
/// `somesite.com` -> `www.somesite.com`. Sources carry no routes of their own but
/// still belong on the certificate.
#[derive(Debug, Clone)]
pub struct RedirectSpec {
    pub from: Vec<String>,
    pub to: String,
    /// 301 when true, 302 when false.
    pub permanent: bool,
}

#[derive(Debug, Clone)]
pub struct IngressTlsSpec {
    pub secret: Option<String>,
    pub letsencrypt: Option<LetsEncryptSpec>,
    /// TLS ends in front of the gateway. Clients still use `https://`, so URLs
    /// and redirects keep that scheme, but the gateway itself serves plain HTTP.
    pub upstream: bool,
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
    /// Per-service overrides, keyed by service name. Empty when the deployment
    /// sets none.
    pub services: HashMap<String, DeploymentServiceSpec>,
    /// Local only: services this deployment does not start.
    pub exclude_services: Vec<String>,
    /// Swarm only: where each named volume lives, keyed by volume name.
    pub volumes: std::collections::BTreeMap<String, VolumeBinding>,
}

#[derive(Debug, Clone)]
pub struct DeploymentSecretSpec {
    pub secret_name: String,
    pub source: DeploymentSecretSource,
}

#[derive(Debug, Clone)]
pub enum DeploymentSecretSource {
    EnvVariable(String),
    /// `secrets_env_prefix`: the deployment names no variable per secret, the
    /// name is derived from the secret's own. Read like `EnvVariable`, except
    /// that the match ignores case — secret names are lower case and environment
    /// variables are not.
    PrefixedEnvVariable(PrefixedEnvSecret),
    FilePath(String),
    Embedded(String),
    /// AWS Secrets Manager. Unlike the other sources this one is *not* read when
    /// the deployment is prepared: the generated artifacts carry the lookup, and
    /// the value is fetched on the machine that runs the deploy. That keeps the
    /// value out of the directory that is shipped to the target.
    Aws(AwsSecretRef),
    /// `json_file:` or `json_env:`: one field of a JSON document. Read when the
    /// deployment is prepared, like `env:` and `file:`, so preparing a sibling
    /// deployment of the same env spec never needs the document.
    JsonField(JsonFieldSecret),
}

#[derive(Debug, Clone)]
pub struct JsonFieldSecret {
    pub document: JsonDocument,
    /// The path the filter addresses, already checked to be a plain field path.
    pub field: Vec<String>,
    /// The filter as written, or the default one, for messages.
    pub filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum JsonDocument {
    /// The full path, and the path as a message prints it.
    File { path: String, display: String },
    /// An environment variable holding the whole document.
    Env(String),
}

impl JsonDocument {
    pub fn describe(&self) -> String {
        match self {
            JsonDocument::File { display, .. } => display.clone(),
            JsonDocument::Env(variable) => format!("${}", variable),
        }
    }
}

/// The value at `field` in `doc` the way jq would print it — a string raw,
/// anything else as its JSON — or `None` when it is absent or `null`.
pub fn select_json_field(doc: &serde_json::Value, field: &[String]) -> Option<String> {
    let mut value = doc;
    for segment in field {
        value = value.get(segment)?;
    }
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// A secret value as it comes out of its source, without the newline whatever
/// wrote it appended. Every source is a file or a command's output — a
/// `secrets_folder` entry, a `file:` source, the AWS CLI — and all of them end
/// the value with a line break that is not part of the credential. That break is
/// load-bearing: a secret mounted as an environment variable cannot carry one at
/// all (`docker run --env-file` reads to the end of the line), so without this a
/// plain `echo "$KEY" > secrets/api_key` fails the whole deployment.
///
/// Only trailing breaks go: a genuinely multi-line secret, a PEM key mounted as
/// a file, keeps its interior newlines.
pub fn trim_secret_value(value: &str) -> &str {
    value.trim_end_matches(['\n', '\r'])
}

/// A secret to look up in the environment under a name it was not given
/// explicitly. See [`DeploymentSecretSource::PrefixedEnvVariable`].
#[derive(Debug, Clone)]
pub struct PrefixedEnvSecret {
    /// `secrets_env_prefix` + the secret's name: the variable to look for.
    pub variable: String,
    /// The sources ruled out before this one — a folder file that is not there, a
    /// `secrets_json` field the document does not have. Carried so that a secret
    /// which is in none of them is reported against all of them at once.
    pub tried: Vec<String>,
    /// `secrets_aws`: where to look when the variable is not set either. The AWS
    /// lookup cannot be attempted where this is decided — for anything but a local
    /// deployment it happens on the deploy target — so it ends the chain.
    pub fallback: Option<AwsSecretRef>,
}

#[derive(Debug, Clone)]
pub struct AwsSecretRef {
    /// Name or ARN, passed to `aws secretsmanager get-secret-value --secret-id`.
    pub secret_id: String,
    /// Optional `jq` filter applied to the fetched `SecretString`, for secrets
    /// that hold a JSON document and only one of its fields is wanted.
    pub jq: Option<String>,
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
    /// Swarm only: rolling update policy. `None` keeps Swarm's defaults.
    pub update: Option<UpdatePolicy>,
    /// Swarm only: placement constraints and preferences.
    pub placement: Option<Placement>,
}

/// `deploy.update_config` of a Swarm service. Only the fields the env spec sets
/// are written; durations are kept as written, already checked to parse.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpdatePolicy {
    pub order: Option<UpdateOrder>,
    pub failure_action: Option<UpdateFailureAction>,
    pub parallelism: Option<u32>,
    pub delay: Option<String>,
    pub monitor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateOrder {
    StartFirst,
    StopFirst,
}

impl UpdateOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateOrder::StartFirst => "start-first",
            UpdateOrder::StopFirst => "stop-first",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateFailureAction {
    Rollback,
    Pause,
    Continue,
}

impl UpdateFailureAction {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateFailureAction::Rollback => "rollback",
            UpdateFailureAction::Pause => "pause",
            UpdateFailureAction::Continue => "continue",
        }
    }
}

/// `deploy.placement` of a Swarm service: constraints such as
/// `node.role == worker` and the label each `spread` preference spreads over.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Placement {
    pub constraints: Vec<String>,
    pub spread: Vec<String>,
}

/// A field is `None` when the env spec does not set it. Kubernetes writes the
/// built-in default in its place; Docker leaves the container unconstrained.
#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    pub memory: Option<String>,
    pub cpu: Option<String>,
}

pub const DEFAULT_MEMORY: &str = "128Mi";
pub const DEFAULT_CPU: &str = "100m";

impl ResourceLimits {
    pub fn memory_or_default(&self) -> &str {
        self.memory.as_deref().unwrap_or(DEFAULT_MEMORY)
    }

    pub fn cpu_or_default(&self) -> &str {
        self.cpu.as_deref().unwrap_or(DEFAULT_CPU)
    }
}

#[derive(Debug, Clone)]
pub struct DeploymentServiceSpec {
    pub variant: Option<String>,
    /// Every (host alias, prefixes) pair this service is served on. The single
    /// `host` + `prefix`/`prefixes` form yields exactly one route; the `hosts`
    /// map yields one per alias, which is how a service reaches two domains
    /// under different paths (a CMS on its own admin domain plus a couple of
    /// prefixes on the site's domain, say). A non-public service has none.
    pub routes: Vec<ServiceRoute>,
    /// Maximum request body, in bytes, for this service's routes. Overrides the
    /// gateway-wide default.
    pub body_limit: Option<u64>,
    pub resources: ResourcesSpec,
    pub ports: Vec<ServicePort>,
    /// Ports the gateway may route to without publishing them on the host.
    /// The upstream port of a service's routes is its first `expose` entry,
    /// then its first published `ports` entry, then 80. Two deployments of the
    /// same app on one server need this: they listen on the same container
    /// port, and publishing it twice is a host port collision.
    pub expose: Vec<String>,
    // Appended to the service's own volumes, never replacing them.
    pub volumes: Vec<ServiceVolume>,
    // Override the app spec's command / entrypoint for this deployment only.
    pub command: Option<ServiceCommand>,
    pub entrypoint: Option<ServiceCommand>,
    // local-only: working directory of a host-run (non-dockerized) service.
    pub working_dir: Option<String>,
    // Replaces the app spec's `depends_on` for this deployment only: the
    // services that must be up first are an environment matter when the same
    // dependency is a stack service in one environment and external in another.
    pub depends_on: Option<Vec<String>>,
}

/// One host alias a service answers on, with the prefixes it claims there.
/// `host` is `None` when the deployment named no alias, meaning the
/// deployment's `primary_host`.
#[derive(Debug, Clone)]
pub struct ServiceRoute {
    pub host: Option<String>,
    pub prefixes: Vec<Prefix>,
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
    fn parses_body_sizes_in_nginx_notation() {
        assert_eq!(parse_body_size_bytes("1024"), Some(1024));
        assert_eq!(parse_body_size_bytes("512k"), Some(512 * 1024));
        assert_eq!(parse_body_size_bytes("10m"), Some(10 * 1024 * 1024));
        assert_eq!(parse_body_size_bytes("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_body_size_bytes(" 2mb "), Some(2 * 1024 * 1024));
        // nginx spells "no limit" as 0, so it is a value rather than an absence.
        assert_eq!(parse_body_size_bytes("0"), Some(0));
    }

    #[test]
    fn rejects_body_sizes_that_are_not_sizes() {
        assert_eq!(parse_body_size_bytes(""), None);
        assert_eq!(parse_body_size_bytes("m"), None);
        assert_eq!(parse_body_size_bytes("10t"), None);
        assert_eq!(parse_body_size_bytes("10 megabytes"), None);
        assert_eq!(parse_body_size_bytes("1.5m"), None);
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
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: false,
        };
        assert_eq!(
            shell.probe_argv(),
            Some(vec!["/bin/sh".into(), "-c".into(), "curl -f localhost".into()])
        );

        let cmd = Healthcheck {
            test: HealthcheckTest::Exec(vec!["CMD".into(), "curl".into(), "-f".into(), "localhost".into()]),
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: false,
        };
        assert_eq!(
            cmd.probe_argv(),
            Some(vec!["curl".into(), "-f".into(), "localhost".into()])
        );

        let cmd_shell = Healthcheck {
            test: HealthcheckTest::Exec(vec!["CMD-SHELL".into(), "curl -f localhost".into()]),
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: false,
        };
        assert_eq!(
            cmd_shell.probe_argv(),
            Some(vec!["/bin/sh".into(), "-c".into(), "curl -f localhost".into()])
        );
    }

    #[test]
    fn disabled_healthcheck_has_no_probe() {
        let by_flag = Healthcheck {
            test: HealthcheckTest::Shell("x".into()),
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: true,
        };
        assert!(by_flag.is_disabled());
        assert_eq!(by_flag.probe_argv(), None);

        let by_none = Healthcheck {
            test: HealthcheckTest::Exec(vec!["NONE".into()]),
            interval: None,
            timeout: None,
            retries: None,
            start_period: None,
            disable: false,
        };
        assert!(by_none.is_disabled());
        assert_eq!(by_none.probe_argv(), None);
    }
}
