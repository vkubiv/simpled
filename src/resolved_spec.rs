use crate::spec::{EnvVariable, ResourcesSpec, ServiceConfigOption, ServicePort, ServiceSecret, ServiceType};

#[derive(Debug)]
pub struct EnvironmentResolvedSpec {
    pub name: String,
    pub ingress: IngressResolvedSpec,
    pub current_deployment: DeploymentResolvedSpec,
}

#[derive(Debug, Clone)]
pub struct IngressResolvedSpec {
    pub name: String,
    pub tls: Option<IngressTlsResolvedSpec>,
    pub domains: Vec<String>,
    pub rules: Vec<IngressRule>,
}

#[derive(Debug, Clone)]
pub struct IngressTlsResolvedSpec {
    pub secret: Option<String>,
    pub letsencrypt: Option<LetsEncryptResolvedSpec>,
}

#[derive(Debug, Clone)]
pub struct LetsEncryptResolvedSpec {
    pub server: String,
    pub email: String,
}

#[derive(Debug, Clone)]
pub struct IngressRule {
    pub domain_name: String,
    pub services: Vec<IngressToServiceRule>,
}

#[derive(Debug, Clone)]
pub struct IngressToServiceRule {
    pub service_name: String,
    pub port: u16,
    pub prefix: String,
    pub strip_prefix: bool,
}

#[derive(Debug)]
pub struct ServiceResolvedSpec {
    pub service_type: ServiceType,

    // name consists of app name + service name
    pub full_name: String,

    // resolved image with a full name, including registry and version
    pub image: String,

    pub service_host: Option<String>,
    // Resolution rules for environment_variables:
    // * AppEnvironment::external are substituted with values from DeploymentSpec::environment
    // * AppEnvironment::relative are transformed into this form https://{service_host}/${variable_value}
    // * AppEnvironment::internal are added without any transformation
    pub environment_variables: Vec<EnvVariable>,
    
    // overrides environment_variables for local non-dockerized execution
    pub undockerized_environment_variables: Vec<EnvVariable>,

    // config_name transformed to app name + original config_name
    pub configs: Vec<ServiceConfigOption>,

    // secret name transformed to app name + original name
    // for k8s deployment values of secrets read from env variable SIMPLED_SECRET_${secret_name}
    pub secrets: Vec<ServiceSecret>,

    pub ports: Vec<ServicePort>,
}

#[derive(Debug)]
pub struct SecretResolvedSpec {
    pub name: String,
    pub value: String,
}

#[derive(Debug)]
pub struct DeploymentResolvedSpec {
    pub name: String,
    pub application_name: String,
    pub configs: Vec<ConfigResolvedSpec>,
    pub secrets: Vec<SecretResolvedSpec>,
    pub defaults: ResourcesSpec,
    pub services: Vec<ServiceResolvedSpec>,
}

#[derive(Debug)]
pub struct ConfigResolvedSpec {
    pub name: String,
    pub files: Vec<ConfigResolvedFile>,
}

#[derive(Debug)]
pub struct ConfigResolvedFile {
    pub name: String,
    pub content: Vec<u8>,
}
