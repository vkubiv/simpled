use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct AppSpecYaml {
    pub name: String,
    pub version: String,
    pub environment: Option<AppEnvironmentYaml>,
    pub app_services: Option<HashMap<String, ServiceSpecYaml>>,
    pub extra_services: Option<HashMap<String, ServiceSpecYaml>>,
    pub configs: Option<HashMap<String, Vec<String>>>,
    pub secrets: Option<AppSecretsYaml>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtraAppSpecYaml {
    pub extra_services: Option<HashMap<String, ServiceSpecYaml>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AppSecretsYaml {
    Simple(Vec<String>),
    Detailed(HashMap<String, Option<serde_yaml::Value>>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppEnvironmentYaml {
    pub external: Option<Vec<String>>,
    pub optional: Option<Vec<String>>,
    pub relative: Option<Vec<String>>,
    pub internal: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportSpecYaml {
    pub host: Option<String>,
    pub prefix: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageVariantYaml {
    pub image: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTypeYaml {
    Public,
    Internal,
    Job,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServiceSecretYaml {
    Simple(String),
    Detailed(HashMap<String, Option<SecretConfigYaml>>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SecretConfigYaml {
    pub path: Option<String>,
    pub variable: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentEnvTypeYaml {
    K8S,
    Docker,
    Local,
}

// DeploymentEnvironmentSpecYaml definitions
#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentEnvironmentSpecYaml {
    #[serde(alias = "type")]
    pub env_type: DeploymentEnvTypeYaml,
    // if env_type is Docker, swarm_mode can be set. In other cases it will cause an error
    pub swarm_mode: Option<bool>,
    pub ingress: IngressSpecYaml,
    pub registry: Option<HashMap<String, String>>,
    pub deployments: HashMap<String, DeploymentSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngressTlsSpecYaml {
    pub secret: Option<String>,
    pub letsencrypt: Option<LetsEncryptSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngressSpecYaml {
    pub name: String,
    pub hosts: HashMap<String, HostSpecYaml>,
    pub tls: Option<IngressTlsSpecYaml>,

    // if env_type is Docker, ingress_type can be nginx or traefik(default). In other cases it will cause an error
    #[serde(rename = "type")]
    pub ingress_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LetsEncryptSpecYaml {
    pub server: Option<String>,
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HostSpecYaml {
    Single(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentSpecYaml {
    pub application: DeploymentAppSpecYaml,
    pub environment: Option<DeploymentEnvVariablesYaml>,
    pub undockerized_environment: Option<DeploymentEnvVariablesYaml>,
    pub configs: Option<HashMap<String, String>>,
    pub secrets: Option<HashMap<String, DeploymentSecretSpecExYaml>>,
    pub defaults: Option<DefaultsSpecYaml>,
    pub services: Option<HashMap<String, DeploymentServiceSpecYaml>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentSecretSpecYaml {
    pub env: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeploymentSecretSpecExYaml {
    Local(String), // for local configurations we can put secrets directly into the deployment spec
    Detailed(DeploymentSecretSpecYaml),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DeploymentEnvVariablesYaml {
    FromEnvFile(String),
    FromList(Vec<String>),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentAppSpecYaml {
    pub name: String,
    pub version: Option<String>,
    pub extra: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DefaultsSpecYaml {
    pub replicas: Option<u32>,
    pub resources: Option<ResourcesSpecYaml>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResourcesSpecYaml {
    pub requests: Option<ResourceLimitsYaml>,
    pub limits: Option<ResourceLimitsYaml>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResourceLimitsYaml {
    pub memory: Option<String>,
    pub cpu: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PrefixOptionsYaml {
    pub strip: Option<bool>,
}
