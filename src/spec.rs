use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AppSpec {
    pub name: String,
    pub version: semver::Version,
    pub environment: AppEnvironment,    
    pub app_services: Vec<ServiceSpec>,
    pub extra_services: Vec<ServiceSpec>,
    pub configs: Vec<ConfigSpec>,
    pub secrets: Vec<AppSecretOption>,
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
pub struct ServiceSpec {
    pub name: String,
    pub service_type: ServiceType,
    pub is_app_service: bool,
    // TODO: redo this logic to have either image or variants
    // create enum ImageSpec { Exact(String), Variants(Vec<ImageVariant>) }
    // change image_variants to image: ImageSpec
    // and return rest of the logic that uses it
    pub image_variants: Vec<ImageVariant>,
    pub environment: Vec<ServiceEnvOption>,
    pub configs: Vec<ServiceConfigOption>,
    pub secrets: Vec<ServiceSecret>,
    pub ports: Vec<ServicePort>,
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
}

#[derive(Debug, Clone)]
pub struct Prefix {
    pub prefix: String,
    pub strip: bool,
}
