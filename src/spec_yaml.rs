use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_gateway_name() -> String {
    "gateway".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSpecYaml {
    pub name: String,
    pub version: String,
    pub environment: Option<AppEnvironmentYaml>,
    pub app_services: Option<HashMap<String, ServiceSpecYaml>>,
    pub extra_services: Option<HashMap<String, ServiceSpecYaml>>,
    pub configs: Option<HashMap<String, Vec<String>>>,
    pub secrets: Option<AppSecretsYaml>,
    pub volumes: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtraAppSpecYaml {
    pub extra_services: Option<HashMap<String, ServiceSpecYaml>>,
    pub environment: Option<AppEnvironmentYaml>,
    pub configs: Option<HashMap<String, Vec<String>>>,
    pub secrets: Option<AppSecretsYaml>,
    pub volumes: Option<Vec<String>>,   
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum AppSecretsYaml {
    Simple(Vec<String>),
    Detailed(HashMap<String, Option<serde_yaml::Value>>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppEnvironmentYaml {
    pub external: Option<Vec<String>>,
    pub optional: Option<Vec<String>>,
    pub relative: Option<Vec<String>>,
    pub internal: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServiceSpecYaml {
    // default is internal
    #[serde(rename = "type")]
    pub service_type: Option<ServiceTypeYaml>,
    pub image: Option<String>,
    pub variants: Option<HashMap<String, ImageVariantYaml>>,
    pub export: Option<ExportSpecYaml>,
    pub environment: Option<Vec<String>>,
    pub configs: Option<Vec<HashMap<String, String>>>,
    pub secrets: Option<Vec<ServiceSecretYaml>>,
    pub ports: Option<Vec<String>>,
    // Internal-only ports, same as docker-compose `expose`.
    pub expose: Option<Vec<String>>,
    pub volumes: Option<Vec<String>>,
    // Overrides the image's default command, same as docker-compose `command`.
    pub command: Option<ServiceCommandYaml>,
    // Overrides the image's ENTRYPOINT, same as docker-compose `entrypoint`.
    pub entrypoint: Option<ServiceCommandYaml>,
    // Container health probe, same as docker-compose `healthcheck`.
    pub healthcheck: Option<HealthcheckYaml>,
    // Services that must be running before this one starts, same as
    // docker-compose `depends_on`. Used to order the phases of a Swarm/standalone
    // deployment: everything a `job` depends on is started before the job runs.
    pub depends_on: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum ServiceCommandYaml {
    Shell(String),
    Exec(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HealthcheckYaml {
    pub test: Option<HealthcheckTestYaml>,
    pub interval: Option<String>,
    pub timeout: Option<String>,
    pub retries: Option<u32>,
    pub start_period: Option<String>,
    pub disable: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum HealthcheckTestYaml {
    Shell(String),
    Exec(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExportSpecYaml {
    pub host: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageVariantYaml {
    pub image: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTypeYaml {
    Public,
    Internal,
    Job,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum ServiceSecretYaml {
    Simple(String),
    Detailed(HashMap<String, Option<SecretConfigYaml>>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SecretConfigYaml {
    pub path: Option<String>,
    pub variable: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentEnvTypeYaml {
    K8S,
    Docker,
    Local,
}

// DeploymentEnvironmentSpecYaml definitions
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploymentEnvironmentSpecYaml {
    #[serde(alias = "type")]
    pub env_type: Option<DeploymentEnvTypeYaml>,
    // if env_type is Docker, swarm_mode can be set. In other cases it will cause an error
    pub swarm_mode: Option<bool>,
    pub gateway: Option<IngressSpecYaml>,
    // deprecated: use gateway instead
    pub ingress: Option<IngressSpecYaml>,
    pub registry: Option<HashMap<String, String>>,
    pub deployments: HashMap<String, DeploymentSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IngressTlsSpecYaml {
    pub disable: Option<bool>,
    pub secret: Option<String>,
    pub letsencrypt: Option<LetsEncryptSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IngressSpecYaml {
    #[serde(default = "default_gateway_name")]
    pub name: String,
    pub hosts: HashMap<String, HostSpecYaml>,
    pub tls: Option<IngressTlsSpecYaml>,

    // if env_type is Docker, ingress_type can be nginx or traefik(default). In other cases it will cause an error
    #[serde(rename = "type")]
    pub ingress_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LetsEncryptSpecYaml {
    pub server: Option<String>,
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum HostSpecYaml {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploymentSpecYaml {
    // Name of another deployment in this env spec to inherit from. Scalar fields
    // (primary_host, application, environment, ...) fall back to the base when not
    // set here; map fields (configs, secrets, services) are merged with the base,
    // and this deployment's entries win on key conflicts. `extends` chains are
    // allowed; cycles are rejected.
    pub extends: Option<String>,
    // Marks a deployment as a template that is only used as an `extends` base. An
    // abstract deployment is not deployed on its own and may omit fields that would
    // otherwise be required (e.g. primary_host, application).
    #[serde(rename = "abstract", default)]
    pub is_abstract: Option<bool>,
    // Required unless inherited via `extends` or the deployment is abstract.
    pub primary_host: Option<String>,
    // Required unless inherited via `extends` or the deployment is abstract.
    pub application: Option<DeploymentAppSpecYaml>,
    pub environment: Option<DeploymentEnvVariablesYaml>,
    pub undockerized_environment: Option<DeploymentEnvVariablesYaml>,
    pub configs: Option<HashMap<String, String>>,
    pub secrets: Option<HashMap<String, DeploymentSecretSpecExYaml>>,
    pub defaults: Option<DefaultsSpecYaml>,
    pub services: Option<HashMap<String, DeploymentServiceSpecYaml>>,
    // local-only: folder to load secret values from when a secret value is empty
    pub secrets_folder: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploymentSecretSpecYaml {
    pub env: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum DeploymentSecretSpecExYaml {
    Local(Option<String>), // for local configurations we can put secrets directly into the deployment spec
    Detailed(DeploymentSecretSpecYaml),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum DeploymentEnvVariablesYaml {
    FromEnvFile(String),
    FromList(Vec<EnvVariableEntryYaml>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum EnvVariableEntryYaml {
    // an inline variable as a string, e.g. "SOME_VAR=some_value"
    Inline(String),
    // a variable whose value is read from a file, e.g.
    //     environment:
    //       - MAIN_SERVICE_DB:
    //           file: /path/to/file
    FromFile(HashMap<String, EnvVariableSourceYaml>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvVariableSourceYaml {
    pub file: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploymentAppSpecYaml {
    pub name: String,
    pub version: Option<String>,
    pub extra: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DefaultsSpecYaml {
    pub replicas: Option<u32>,
    pub resources: Option<ResourcesSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourcesSpecYaml {
    pub requests: Option<ResourceLimitsYaml>,
    pub limits: Option<ResourceLimitsYaml>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResourceLimitsYaml {
    pub memory: Option<String>,
    pub cpu: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeploymentServiceSpecYaml {
    pub variant: Option<String>,
    pub host: Option<String>,
    pub prefix: Option<String>,
    pub strip_prefix: Option<bool>,
    pub prefixes: Option<HashMap<String, PrefixOptionsYaml>>,
    pub replicas: Option<u32>,
    pub resources: Option<ResourcesSpecYaml>,
    // ports are a vector of strings in the form "external:internal"
    pub ports: Option<Vec<String>>,

    // local-only: directory of a host-run (non-dockerized) service. When set,
    // the undockerized environment is written there as `.env` and the service's
    // secrets are copied alongside it. Setting it for K8S/Docker is an error.
    pub working_dir: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PrefixOptionsYaml {
    pub strip: Option<bool>,
}
