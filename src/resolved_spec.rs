use crate::spec::{DeploymentEnvType, EnvVariable, Healthcheck, ResourcesSpec, ServiceCommand, ServiceConfigOption, ServicePort, ServiceSecret, ServiceType, ServiceVolume};

#[derive(Debug)]
pub struct EnvironmentResolvedSpec {
    pub env_type: DeploymentEnvType,
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
    pub deployment_name: String,
    pub port: u16,
    pub prefix: String,
    pub strip_prefix: bool,
}

#[derive(Debug)]
pub struct ServiceResolvedSpec {
    pub service_type: ServiceType,
    pub is_app_service: bool,

    // name consists of service name
    pub full_name: String,

    // resolved image with a full name, including registry and version
    pub image: String,

    pub service_host: String,
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

    // Internal-only ports (not published to the host), same as docker-compose `expose`.
    pub expose: Vec<String>,

    pub volumes: Vec<ServiceVolume>,

    // Overrides the image's default command, same as docker-compose `command`.
    pub command: Option<ServiceCommand>,

    // Overrides the image's ENTRYPOINT, same as docker-compose `entrypoint`.
    pub entrypoint: Option<ServiceCommand>,

    // Container health probe, same as docker-compose `healthcheck`.
    pub healthcheck: Option<Healthcheck>,

    // full_names of the services that must be running before this one starts.
    pub depends_on: Vec<String>,

    // local-only: working directory of a host-run (non-dockerized) service.
    // When set, undockerized env is written there as `.env` and secrets copied alongside.
    pub working_dir: Option<String>,
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
    pub volumes: Vec<String>,
}

impl DeploymentResolvedSpec {
    /// Services that run to completion (`type: job`), in dependency order: a job
    /// that depends on another job comes after it. Ties are broken by name so the
    /// generated scripts are stable across runs.
    pub fn jobs_in_order(&self) -> Vec<&ServiceResolvedSpec> {
        let mut jobs: Vec<&ServiceResolvedSpec> = self.services.iter()
            .filter(|s| matches!(s.service_type, ServiceType::Job))
            .collect();
        jobs.sort_by(|a, b| a.full_name.cmp(&b.full_name));

        let mut ordered: Vec<&ServiceResolvedSpec> = Vec::new();
        let mut placed: Vec<&str> = Vec::new();
        for job in &jobs {
            self.place_job(job, &jobs, &mut ordered, &mut placed);
        }
        ordered
    }

    fn place_job<'a>(
        &'a self,
        job: &'a ServiceResolvedSpec,
        jobs: &[&'a ServiceResolvedSpec],
        ordered: &mut Vec<&'a ServiceResolvedSpec>,
        placed: &mut Vec<&'a str>,
    ) {
        if placed.contains(&job.full_name.as_str()) {
            return;
        }
        // Marked before recursing so a cycle (rejected by the validator, but the
        // generator must not hang on one) cannot loop forever.
        placed.push(&job.full_name);
        for dep in &job.depends_on {
            if let Some(dep_job) = jobs.iter().find(|j| &j.full_name == dep) {
                self.place_job(dep_job, jobs, ordered, placed);
            }
        }
        ordered.push(job);
    }

    /// Long-running services (everything that is not a job), sorted by name.
    pub fn long_running_services(&self) -> Vec<&ServiceResolvedSpec> {
        let mut services: Vec<&ServiceResolvedSpec> = self.services.iter()
            .filter(|s| !matches!(s.service_type, ServiceType::Job))
            .collect();
        services.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        services
    }

    /// Long-running services that must be up before any job runs: the transitive
    /// closure of `depends_on` over every job, with jobs themselves removed.
    ///
    /// A job that declares no `depends_on` is treated as depending on every
    /// long-running service. Without that fallback, upgrading to phased deploys
    /// would start running jobs before the database exists for specs written
    /// before `depends_on` existed.
    pub fn job_prerequisites(&self) -> Vec<&ServiceResolvedSpec> {
        let jobs = self.jobs_in_order();
        if jobs.is_empty() {
            return Vec::new();
        }

        if jobs.iter().any(|j| j.depends_on.is_empty()) {
            return self.long_running_services();
        }

        let mut needed: Vec<&str> = Vec::new();
        for job in &jobs {
            for dep in &job.depends_on {
                self.collect_dependencies(dep, &mut needed);
            }
        }

        let mut services: Vec<&ServiceResolvedSpec> = self.services.iter()
            .filter(|s| !matches!(s.service_type, ServiceType::Job))
            .filter(|s| needed.contains(&s.full_name.as_str()))
            .collect();
        services.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        services
    }

    fn collect_dependencies<'a>(&'a self, name: &str, needed: &mut Vec<&'a str>) {
        let Some(service) = self.services.iter().find(|s| s.full_name == name) else {
            return;
        };
        if needed.contains(&service.full_name.as_str()) {
            return;
        }
        needed.push(&service.full_name);
        for dep in &service.depends_on {
            self.collect_dependencies(dep, needed);
        }
    }
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
