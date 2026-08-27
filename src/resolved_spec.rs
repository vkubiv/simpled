use crate::spec::{AwsSecretRef, DeploymentEnvType, EnvVariable, Healthcheck, ResourcesSpec, ServiceCommand, ServiceConfigOption, ServicePort, ServiceSecret, ServiceType, ServiceVolume};

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
    /// Every domain the gateway answers on, redirect sources included, so that a
    /// certificate covers the redirect too.
    pub domains: Vec<String>,
    pub rules: Vec<IngressRule>,
    pub redirects: Vec<RedirectRule>,
}

/// One source domain bounced to one destination. The spec allows several sources
/// per entry; the resolver flattens them so every generator sees a flat list.
#[derive(Debug, Clone)]
pub struct RedirectRule {
    pub from_domain: String,
    /// Either a bare domain or a full URL, exactly as written in the spec.
    pub to: String,
    /// 301 when true, 302 when false.
    pub permanent: bool,
}

impl RedirectRule {
    /// Destination as an absolute URL without a trailing slash. A bare domain in
    /// the spec picks up the gateway's own scheme, so an environment that
    /// terminates TLS redirects to `https://`.
    pub fn target_url(&self, has_tls: bool) -> String {
        let to = self.to.trim_end_matches('/');
        if to.contains("://") {
            to.to_string()
        } else if has_tls {
            format!("https://{}", to)
        } else {
            format!("http://{}", to)
        }
    }

    pub fn status_code(&self) -> u16 {
        if self.permanent { 301 } else { 302 }
    }
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
    pub value: SecretResolvedValue,
}

/// Where a secret's value comes from once the deployment has been resolved.
#[derive(Debug, Clone)]
pub enum SecretResolvedValue {
    /// Known while the deployment is prepared, and baked into the generated files.
    Literal(String),
    /// Only known on the machine that runs the deploy. The generators emit the
    /// lookup into `fetch-secrets.sh` instead of the value itself.
    Deferred(AwsSecretRef),
}

impl SecretResolvedSpec {
    /// The value to write into a generated file, or `None` when it is only
    /// available on the deploy target.
    pub fn literal(&self) -> Option<&str> {
        match &self.value {
            SecretResolvedValue::Literal(value) => Some(value),
            SecretResolvedValue::Deferred(_) => None,
        }
    }

    pub fn deferred(&self) -> Option<&AwsSecretRef> {
        match &self.value {
            SecretResolvedValue::Literal(_) => None,
            SecretResolvedValue::Deferred(reference) => Some(reference),
        }
    }

    /// Shell variable `fetch-secrets.sh` exports the fetched value as. Used to
    /// reference a deferred secret from the generated deploy script and, through
    /// compose's `${VAR}` interpolation, from the generated stack file.
    pub fn shell_var(&self) -> String {
        let sanitized: String = self.name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' })
            .collect();
        format!("{}{}", SHELL_VAR_PREFIX, sanitized)
    }
}

/// Prefix of the shell variables that carry deferred secret values. Also used to
/// recognize a placeholder again when the compose representation of a service is
/// translated back into `docker service create` flags for a job.
pub const SHELL_VAR_PREFIX: &str = "SIMPLED_SECRET_";

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
    /// Secrets whose value is only fetched on the deploy target, paired with the
    /// AWS lookup that fetches it. Empty for a deployment that declares none, in
    /// which case no `fetch-secrets.sh` is generated at all.
    pub fn deferred_secrets(&self) -> Vec<(&SecretResolvedSpec, &AwsSecretRef)> {
        self.secrets.iter()
            .filter_map(|secret| secret.deferred().map(|reference| (secret, reference)))
            .collect()
    }

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
