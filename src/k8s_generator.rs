use crate::resolved_spec::ServiceResolvedSpec;
use crate::resolved_spec::{
    EnvironmentResolvedSpec, IngressResolvedSpec, IngressToServiceRule, LetsEncryptResolvedSpec,
};
use crate::secret_fetch::{self, sh_quote, FetchScript};
use crate::spec::{parse_duration_secs, Healthcheck, SecretMount, ServiceType};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const LETSENCRYPT_ISSUER: &str = "letsencrypt-prod";

pub fn generate(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }

    let deployment = &resolved_spec.current_deployment;

    // 1. ConfigMaps
    for config in &deployment.configs {
        let name = k8s_name(&config.name);
        let file_name = output_dir.join(format!("configmap-{}.yaml", name));
        let mut file = File::create(file_name)?;
        writeln!(file, "apiVersion: v1")?;
        writeln!(file, "kind: ConfigMap")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", name)?;
        writeln!(file, "binaryData:")?;
        for cfg_file in &config.files {
            let encoded = general_purpose::STANDARD.encode(&cfg_file.content);
            writeln!(file, "  {}: {}", cfg_file.name, encoded)?;
        }
    }

    // 2. Secrets. A secret with an `aws` source gets no manifest: its value is
    // only read where the deploy runs, so `fetch-secrets.sh` applies it with
    // kubectl instead of shipping it base64-encoded in the manifest directory.
    let mut fetch_script = FetchScript::new();
    for secret in &deployment.secrets {
        if let Some(reference) = secret.deferred() {
            fetch_script.fetch(secret, reference);
        }
    }
    // Every lookup runs before the first `kubectl apply`, so a secret that does not
    // resolve leaves none of the others already applied to the cluster.
    for secret in &deployment.secrets {
        if secret.deferred().is_some() {
            fetch_script.command(format!(
                "kubectl create secret generic {name} --from-literal=value=\"${var}\" \
                 --dry-run=client -o yaml | kubectl apply -f -",
                name = sh_quote(&k8s_name(&secret.name)),
                var = secret.shell_var(),
            ));
        }
    }
    for secret in &deployment.secrets {
        if secret.deferred().is_some() {
            continue;
        }

        let name = k8s_name(&secret.name);
        let file_name = output_dir.join(format!("secret-{}.yaml", name));
        let mut file = File::create(file_name)?;
        writeln!(file, "apiVersion: v1")?;
        writeln!(file, "kind: Secret")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", name)?;
        writeln!(file, "type: Opaque")?;
        writeln!(file, "data:")?;
        let encoded = general_purpose::STANDARD.encode(secret.literal().unwrap_or_default());
        writeln!(file, "  value: {}", encoded)?;
    }

    if !fetch_script.is_empty() {
        fetch_script.write(
            &output_dir.join(secret_fetch::SCRIPT_NAME),
            "Run it against the target cluster before `kubectl apply -f .`; it creates the \
             Secrets the manifests here reference.",
        )?;
    }

    // 3. Workloads & Services
    let deploy_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    for service in &deployment.services {
        write_workload(output_dir, service, deploy_date)?;
        write_service(output_dir, service)?;
    }

    // 4. Ingress
    generate_ingress(resolved_spec, output_dir)?;

    // 5. ClusterIssuer (if needed)
    if let Some(tls) = &resolved_spec.ingress.tls {
        if let Some(le) = &tls.letsencrypt {
            generate_cluster_issuer(output_dir, le)?;
        }
    }

    Ok(())
}

/// Seconds a finished Job is kept before Kubernetes garbage-collects it, long
/// enough to read its logs after a failed deploy.
const JOB_TTL_SECONDS: u32 = 86_400;

/// Kubernetes object names must be DNS-1123 subdomains: lower-case alphanumerics,
/// `-` and `.`, starting and ending with an alphanumeric. Spec names are freer
/// (`db_password` is the documented style), so every name is normalized on the
/// way into a manifest, references included, and both sides always agree.
pub fn k8s_name(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '-' })
        .collect();
    mapped.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_string()
}

/// A YAML double-quoted scalar, so a value holding quotes, backslashes or line
/// breaks cannot break the manifest around it.
fn yaml_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The Deployment for a long-running service, or the Job for a `type: job` one.
///
/// A Job's pod template is immutable and its image changes with every release,
/// so re-applying a Job of the same name would be rejected. Each deploy therefore
/// gets a Job of its own, named after the deploy date and garbage-collected a day
/// after it finishes. Kubernetes applies the whole directory at once, so unlike
/// the Docker targets nothing waits for the job: `depends_on` is not enforced.
fn write_workload(output_dir: &Path, service: &ServiceResolvedSpec, deploy_date: u64) -> Result<()> {
    let name = k8s_name(&service.full_name);
    let is_job = matches!(service.service_type, ServiceType::Job);

    let file_name = if is_job {
        output_dir.join(format!("job-{}.yaml", name))
    } else {
        output_dir.join(format!("deployment-{}.yaml", name))
    };
    let mut file = File::create(file_name)?;

    if is_job {
        writeln!(file, "apiVersion: batch/v1")?;
        writeln!(file, "kind: Job")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}-{}", name, deploy_date)?;
        writeln!(file, "  labels:")?;
        writeln!(file, "    app: {}", name)?;
        writeln!(file, "spec:")?;
        // A migration that fails must surface as a failed deploy, not be retried
        // until it happens to pass; this mirrors the Docker deploy scripts.
        writeln!(file, "  backoffLimit: 0")?;
        writeln!(file, "  ttlSecondsAfterFinished: {}", JOB_TTL_SECONDS)?;
        writeln!(file, "  template:")?;
        writeln!(file, "    metadata:")?;
        writeln!(file, "      labels:")?;
        writeln!(file, "        app: {}", name)?;
        writeln!(file, "    spec:")?;
        writeln!(file, "      restartPolicy: Never")?;
    } else {
        writeln!(file, "apiVersion: apps/v1")?;
        writeln!(file, "kind: Deployment")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", name)?;
        writeln!(file, "spec:")?;
        writeln!(file, "  replicas: {}", service.resources.replicas)?;
        writeln!(file, "  selector:")?;
        writeln!(file, "    matchLabels:")?;
        writeln!(file, "      app: {}", name)?;
        writeln!(file, "  template:")?;
        writeln!(file, "    metadata:")?;
        writeln!(file, "      labels:")?;
        writeln!(file, "        app: {}", name)?;
        writeln!(file, "    spec:")?;
    }

    writeln!(file, "      containers:")?;
    writeln!(file, "      - name: {}", name)?;
    writeln!(file, "        image: {}", service.image)?;
    // docker-compose `entrypoint` overrides the image ENTRYPOINT, which maps
    // to a container's `command` in Kubernetes; `command` overrides the image
    // CMD, which maps to a container's `args`.
    if let Some(entrypoint) = &service.entrypoint {
        writeln!(file, "        command:")?;
        for arg in entrypoint.to_args() {
            writeln!(file, "        - {}", yaml_quote(&arg))?;
        }
    }
    if let Some(command) = &service.command {
        writeln!(file, "        args:")?;
        for arg in command.to_args() {
            writeln!(file, "        - {}", yaml_quote(&arg))?;
        }
    }
    // docker-compose `healthcheck` maps to liveness/readiness probes. A job is
    // expected to exit, so it gets neither.
    if !is_job {
        if let Some(hc) = &service.healthcheck {
            if let Some(argv) = hc.probe_argv() {
                write_probe(&mut file, "livenessProbe", &argv, hc)?;
                write_probe(&mut file, "readinessProbe", &argv, hc)?;
            }
        }
    }
    writeln!(file, "        resources:")?;
    writeln!(file, "          requests:")?;
    writeln!(file, "            memory: {}", service.resources.requests.memory)?;
    writeln!(file, "            cpu: {}", service.resources.requests.cpu)?;
    writeln!(file, "          limits:")?;
    writeln!(file, "            memory: {}", service.resources.limits.memory)?;
    writeln!(file, "            cpu: {}", service.resources.limits.cpu)?;

    writeln!(file, "        env:")?;
    writeln!(file, "        - name: DEPLOY_DATE")?;
    writeln!(file, "          value: \"{}\"", deploy_date)?;
    for env in &service.environment_variables {
        writeln!(file, "        - name: {}", env.name)?;
        writeln!(file, "          value: {}", yaml_quote(&env.value))?;
    }
    for secret in &service.secrets {
        if let SecretMount::EnvVariable(var_name) = &secret.mount {
            writeln!(file, "        - name: {}", var_name)?;
            writeln!(file, "          valueFrom:")?;
            writeln!(file, "            secretKeyRef:")?;
            writeln!(file, "              name: {}", k8s_name(&secret.name))?;
            writeln!(file, "              key: value")?;
        }
    }

    // Volume Mounts (Configs & Secrets)
    let has_volume_mounts = !service.configs.is_empty()
        || service
            .secrets
            .iter()
            .any(|s| !matches!(s.mount, SecretMount::EnvVariable(_)));

    if has_volume_mounts {
        writeln!(file, "        volumeMounts:")?;
        for config in &service.configs {
            writeln!(file, "        - name: config-{}", k8s_name(&config.config_name))?;
            writeln!(file, "          mountPath: {}", config.mount_path)?;
        }
        for secret in &service.secrets {
            if let SecretMount::FilePath(path) = &secret.mount {
                writeln!(file, "        - name: secret-{}", k8s_name(&secret.name))?;
                writeln!(file, "          mountPath: {}", path)?;
            }
        }

        writeln!(file, "      volumes:")?;
        for config in &service.configs {
            let config_name = k8s_name(&config.config_name);
            writeln!(file, "      - name: config-{}", config_name)?;
            writeln!(file, "        configMap:")?;
            writeln!(file, "          name: {}", config_name)?;
        }
        for secret in &service.secrets {
            if !matches!(secret.mount, SecretMount::EnvVariable(_)) {
                let secret_name = k8s_name(&secret.name);
                writeln!(file, "      - name: secret-{}", secret_name)?;
                writeln!(file, "        secret:")?;
                writeln!(file, "          secretName: {}", secret_name)?;
            }
        }
    }

    Ok(())
}

/// The Service in front of a long-running workload. A Service must declare at
/// least one port, and a job is never dialled, so neither gets one.
fn write_service(output_dir: &Path, service: &ServiceResolvedSpec) -> Result<()> {
    if matches!(service.service_type, ServiceType::Job) || service.ports.is_empty() {
        return Ok(());
    }
    let name = k8s_name(&service.full_name);
    let svc_file_name = output_dir.join(format!("service-{}.yaml", name));
    let mut svc_file = File::create(svc_file_name)?;
    writeln!(svc_file, "apiVersion: v1")?;
    writeln!(svc_file, "kind: Service")?;
    writeln!(svc_file, "metadata:")?;
    writeln!(svc_file, "  name: {}", name)?;
    writeln!(svc_file, "spec:")?;
    writeln!(svc_file, "  selector:")?;
    writeln!(svc_file, "    app: {}", name)?;
    writeln!(svc_file, "  ports:")?;
    for port in &service.ports {
        writeln!(svc_file, "  - port: {}", port.external)?;
        writeln!(svc_file, "    targetPort: {}", port.internal)?;
    }
    Ok(())
}

/// Write a Kubernetes exec probe (`livenessProbe`/`readinessProbe`) built from a
/// docker-compose healthcheck. Compose durations are parsed to whole seconds;
/// `retries` maps to `failureThreshold` and `start_period` to
/// `initialDelaySeconds`. Fields with no compose counterpart are left to the
/// Kubernetes defaults.
fn write_probe(file: &mut File, name: &str, argv: &[String], hc: &Healthcheck) -> Result<()> {
    writeln!(file, "        {}:", name)?;
    writeln!(file, "          exec:")?;
    writeln!(file, "            command:")?;
    for arg in argv {
        writeln!(file, "            - {}", yaml_quote(arg))?;
    }
    if let Some(interval) = hc.interval.as_deref().and_then(parse_duration_secs) {
        writeln!(file, "          periodSeconds: {}", interval)?;
    }
    if let Some(timeout) = hc.timeout.as_deref().and_then(parse_duration_secs) {
        writeln!(file, "          timeoutSeconds: {}", timeout)?;
    }
    if let Some(retries) = hc.retries {
        writeln!(file, "          failureThreshold: {}", retries)?;
    }
    if let Some(start) = hc.start_period.as_deref().and_then(parse_duration_secs) {
        writeln!(file, "          initialDelaySeconds: {}", start)?;
    }
    Ok(())
}

fn generate_ingress(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path) -> Result<()> {
    let file_name = output_dir.join("ingress.yaml");
    let mut file = File::create(file_name)?;
    let ingress = &resolved_spec.ingress;

    // `proxy-body-size` is an annotation, and annotations cover a whole Ingress
    // rather than a single path, so routes that want different limits cannot
    // share one object. Group them by limit and emit an Ingress per group; with
    // a single limit (the usual case) that is still just one object.
    let mut limits: Vec<Option<u64>> = Vec::new();
    let mut services_by_limit: HashMap<Option<u64>, Vec<(&String, &IngressToServiceRule)>> = HashMap::new();
    for rule in &ingress.rules {
        for svc in &rule.services {
            if !services_by_limit.contains_key(&svc.body_limit) {
                limits.push(svc.body_limit);
            }
            services_by_limit
                .entry(svc.body_limit)
                .or_default()
                .push((&rule.domain_name, svc));
        }
    }
    // A gateway with no routes at all still emits its (empty) Ingress, as before.
    if limits.is_empty() {
        limits.push(None);
        services_by_limit.insert(None, Vec::new());
    }
    // Which group is written first decides which object keeps the gateway's name
    // and owns the certificate, so it must not depend on the order routes happen
    // to arrive in. `None` sorts before `Some`, so an unlimited group leads.
    limits.sort();

    for (index, limit) in limits.iter().enumerate() {
        // Named after the limit rather than the position, so adding a third group
        // later cannot rename an object that is already deployed.
        let limit_suffix = limit.map(|l| l.to_string()).unwrap_or_else(|| "default".to_string());
        // The first group keeps the gateway's own name and owns the certificate.
        // The rest only add paths to hosts the first one already serves, so they
        // need neither a tls block nor the issuer annotation - a second
        // Certificate would just race the first for the same secret.
        let primary = index == 0;
        let services = &services_by_limit[limit];

        if !primary {
            writeln!(file, "---")?;
        }
        writeln!(file, "apiVersion: networking.k8s.io/v1")?;
        writeln!(file, "kind: Ingress")?;
        writeln!(file, "metadata:")?;
        if primary {
            writeln!(file, "  name: {}", ingress.name)?;
        } else {
            writeln!(file, "  name: {}--limit-{}", ingress.name, limit_suffix)?;
        }
        writeln!(file, "  annotations:")?;
        if primary {
            if let Some(tls) = &ingress.tls {
                if tls.letsencrypt.is_some() {
                    writeln!(file, "    cert-manager.io/cluster-issuer: {}", LETSENCRYPT_ISSUER)?;
                }
            }
        }
        // rewrite-target is per-Ingress too, so it follows the group.
        if services.iter().any(|(_, svc)| svc.strip_prefix) {
            writeln!(file, "    nginx.ingress.kubernetes.io/rewrite-target: /$2")?;
        }
        if let Some(limit) = limit {
            writeln!(file, "    nginx.ingress.kubernetes.io/proxy-body-size: \"{}\"", limit)?;
        }

        writeln!(file, "spec:")?;
        writeln!(file, "  ingressClassName: nginx")?;
        if primary {
            if let Some(tls) = &ingress.tls {
                writeln!(file, "  tls:")?;
                writeln!(file, "  - hosts:")?;
                // The same domain can be declared under multiple host groups, so
                // `ingress.domains` may contain duplicates; emit each host only once.
                let mut seen_hosts: Vec<&String> = Vec::new();
                for domain in &ingress.domains {
                    if seen_hosts.contains(&domain) {
                        continue;
                    }
                    seen_hosts.push(domain);
                    writeln!(file, "    - {}", domain)?;
                }
                if let Some(secret) = &tls.secret {
                    writeln!(file, "    secretName: {}", secret)?;
                } else if tls.letsencrypt.is_some() {
                    writeln!(file, "    secretName: {}--tls", ingress.name)?;
                }
            }
        }

        writeln!(file, "  rules:")?;

        // The same domain can be declared under multiple host groups, producing
        // several rules with the same domain_name. Emit one `- host:` entry per
        // domain with all of its services merged, rather than repeating the host.
        let mut domains: Vec<&String> = Vec::new();
        let mut services_by_domain: HashMap<&String, Vec<&IngressToServiceRule>> = HashMap::new();
        for (domain, svc) in services {
            if !services_by_domain.contains_key(domain) {
                domains.push(domain);
            }
            services_by_domain.entry(domain).or_default().push(svc);
        }

        for domain in domains {
            writeln!(file, "  - host: {}", domain)?;
            writeln!(file, "    http:")?;
            writeln!(file, "      paths:")?;

            for svc_rule in &services_by_domain[domain] {
                let path = if svc_rule.strip_prefix {
                    let trimmed = svc_rule.prefix.trim_end_matches('/');
                    format!("{}(/|$)(.*)", trimmed)
                } else {
                    svc_rule.prefix.clone()
                };

                let path_type = if svc_rule.strip_prefix {
                    "ImplementationSpecific"
                } else {
                    "Prefix"
                };
                writeln!(file, "      - path: {}", path)?;
                writeln!(file, "        pathType: {}", path_type)?;
                writeln!(file, "        backend:")?;
                writeln!(file, "          service:")?;
                writeln!(file, "            name: {}", k8s_name(&svc_rule.service_name))?;
                writeln!(file, "            port:")?;
                writeln!(file, "              number: {}", svc_rule.port)?;
            }
        }
    }

    generate_redirect_ingresses(&resolved_spec.ingress, &mut file)?;

    Ok(())
}

/// Ingress objects for domains that only bounce the client elsewhere, e.g.
/// `somesite.com` -> `www.somesite.com`.
///
/// ingress-nginx configures a redirect through an annotation, and annotations
/// apply to a whole Ingress rather than to a single rule, so each distinct
/// destination needs its own object. Sources sharing a destination are grouped
/// into one.
fn generate_redirect_ingresses(ingress: &IngressResolvedSpec, file: &mut File) -> Result<()> {
    if ingress.redirects.is_empty() {
        return Ok(());
    }

    // A path still has to name a backend even though the redirect answers before
    // the request reaches it. Borrow the first routed service instead of
    // inventing a name that resolves to nothing.
    let placeholder = ingress.rules.iter()
        .flat_map(|rule| rule.services.iter())
        .next()
        .ok_or_else(|| anyhow!(
            "Gateway declares redirects but no routed services; a Kubernetes redirect needs at least one service to attach to"
        ))?;

    // Group by destination, preserving first-seen order so the output is stable.
    let mut targets: Vec<(&str, bool)> = Vec::new();
    let mut sources_by_target: HashMap<(&str, bool), Vec<&str>> = HashMap::new();
    for redirect in &ingress.redirects {
        let key = (redirect.to.as_str(), redirect.permanent);
        if !sources_by_target.contains_key(&key) {
            targets.push(key);
        }
        sources_by_target.entry(key).or_default().push(&redirect.from_domain);
    }

    let has_tls = ingress.tls.is_some();
    // The main Ingress already lists every domain, redirect sources included, in
    // its own `tls` block, so the certificate it owns covers these hosts too.
    // Referencing that secret here (without the issuer annotation) avoids a
    // second Certificate racing the first one for the same secret.
    let tls_secret = ingress.tls.as_ref().and_then(|tls| {
        tls.secret
            .clone()
            .or_else(|| tls.letsencrypt.as_ref().map(|_| format!("{}--tls", ingress.name)))
    });

    for (index, key) in targets.iter().enumerate() {
        let (to, permanent) = *key;
        let sources = &sources_by_target[key];
        let target_url = ingress
            .redirects
            .iter()
            .find(|r| r.to == to && r.permanent == permanent)
            .map(|r| r.target_url(has_tls))
            .unwrap_or_else(|| to.to_string());

        writeln!(file, "---")?;
        writeln!(file, "apiVersion: networking.k8s.io/v1")?;
        writeln!(file, "kind: Ingress")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}--redirect-{}", ingress.name, index)?;
        writeln!(file, "  annotations:")?;
        let annotation = if permanent {
            "permanent-redirect"
        } else {
            "temporal-redirect"
        };
        writeln!(file, "    nginx.ingress.kubernetes.io/{}: {}", annotation, target_url)?;
        writeln!(file, "spec:")?;
        writeln!(file, "  ingressClassName: nginx")?;
        if let Some(secret) = &tls_secret {
            writeln!(file, "  tls:")?;
            writeln!(file, "  - hosts:")?;
            for source in sources {
                writeln!(file, "    - {}", source)?;
            }
            writeln!(file, "    secretName: {}", secret)?;
        }
        writeln!(file, "  rules:")?;
        for source in sources {
            writeln!(file, "  - host: {}", source)?;
            writeln!(file, "    http:")?;
            writeln!(file, "      paths:")?;
            writeln!(file, "      - path: /")?;
            writeln!(file, "        pathType: Prefix")?;
            writeln!(file, "        backend:")?;
            writeln!(file, "          service:")?;
            writeln!(file, "            name: {}", k8s_name(&placeholder.service_name))?;
            writeln!(file, "            port:")?;
            writeln!(file, "              number: {}", placeholder.port)?;
        }
    }

    Ok(())
}

fn generate_cluster_issuer(output_dir: &Path, le_spec: &LetsEncryptResolvedSpec) -> Result<()> {
    let file_name = output_dir.join("cluster-issuer.yaml");
    let mut file = File::create(file_name)?;

    writeln!(file, "apiVersion: cert-manager.io/v1")?;
    writeln!(file, "kind: ClusterIssuer")?;
    writeln!(file, "metadata:")?;
    writeln!(file, "  name: {}", LETSENCRYPT_ISSUER)?;
    writeln!(file, "spec:")?;
    writeln!(file, "  acme:")?;
    writeln!(file, "    server: {}", le_spec.server)?;
    writeln!(file, "    email: {}", le_spec.email)?;
    writeln!(file, "    privateKeySecretRef:")?;
    writeln!(file, "      name: {}", LETSENCRYPT_ISSUER)?;
    writeln!(file, "    solvers:")?;
    writeln!(file, "    - http01:")?;
    writeln!(file, "        ingress:")?;
    writeln!(file, "          class: nginx")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolved_spec::{IngressRule, IngressTlsResolvedSpec, RedirectRule};
    use std::fs;

    fn ingress(redirects: Vec<RedirectRule>, tls: Option<IngressTlsResolvedSpec>) -> IngressResolvedSpec {
        IngressResolvedSpec {
            name: "gateway".to_string(),
            tls,
            domains: vec!["www.somesite.com".to_string()],
            rules: vec![IngressRule {
                domain_name: "www.somesite.com".to_string(),
                services: vec![IngressToServiceRule {
                    service_name: "api".to_string(),
                    deployment_name: "prod".to_string(),
                    port: 8080,
                    prefix: "/".to_string(),
                    strip_prefix: false,
                    body_limit: None,
                }],
            }],
            redirects,
        }
    }

    fn redirect(from: &str, to: &str, permanent: bool) -> RedirectRule {
        RedirectRule {
            from_domain: from.to_string(),
            to: to.to_string(),
            permanent,
        }
    }

    fn render(ingress: &IngressResolvedSpec) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ingress.yaml");
        {
            let mut file = File::create(&path).unwrap();
            generate_redirect_ingresses(ingress, &mut file).unwrap();
        }
        fs::read_to_string(path).unwrap()
    }

    fn spec_with_limits(limits: &[(&str, &str, Option<u64>)]) -> EnvironmentResolvedSpec {
        EnvironmentResolvedSpec {
            env_type: crate::spec::DeploymentEnvType::K8S,
            ingress: IngressResolvedSpec {
                name: "gateway".to_string(),
                tls: Some(IngressTlsResolvedSpec {
                    secret: Some("tls".to_string()),
                    letsencrypt: None,
                }),
                domains: vec!["somesite.com".to_string()],
                rules: vec![IngressRule {
                    domain_name: "somesite.com".to_string(),
                    services: limits
                        .iter()
                        .map(|(name, prefix, limit)| IngressToServiceRule {
                            service_name: name.to_string(),
                            deployment_name: "prod".to_string(),
                            port: 80,
                            prefix: prefix.to_string(),
                            strip_prefix: false,
                            body_limit: *limit,
                        })
                        .collect(),
                }],
                redirects: vec![],
            },
            current_deployment: crate::resolved_spec::DeploymentResolvedSpec {
                name: "prod".to_string(),
                application_name: "shop".to_string(),
                configs: vec![],
                secrets: vec![],
                services: vec![],
                volumes: vec![],
            },
        }
    }

    fn render_ingress(spec: &EnvironmentResolvedSpec) -> String {
        let dir = tempfile::tempdir().unwrap();
        generate_ingress(spec, dir.path()).unwrap();
        fs::read_to_string(dir.path().join("ingress.yaml")).unwrap()
    }

    #[test]
    fn one_shared_limit_stays_a_single_ingress() {
        let yaml = render_ingress(&spec_with_limits(&[
            ("api", "/api", Some(2048)),
            ("web", "/", Some(2048)),
        ]));

        assert_eq!(yaml.matches("kind: Ingress").count(), 1, "{}", yaml);
        assert!(
            yaml.contains("nginx.ingress.kubernetes.io/proxy-body-size: \"2048\""),
            "{}",
            yaml
        );
    }

    #[test]
    fn differing_limits_split_into_separate_ingresses() {
        let yaml = render_ingress(&spec_with_limits(&[
            ("web", "/", None),
            ("upload", "/upload", Some(10 * 1024 * 1024)),
        ]));

        // proxy-body-size is a per-Ingress annotation, so the routes cannot share
        // one object.
        assert_eq!(yaml.matches("kind: Ingress").count(), 2, "{}", yaml);
        assert!(
            yaml.contains(
                "  name: gateway
"
            ),
            "{}",
            yaml
        );
        assert!(yaml.contains("  name: gateway--limit-10485760"), "{}", yaml);
        assert!(
            yaml.contains("nginx.ingress.kubernetes.io/proxy-body-size: \"10485760\""),
            "{}",
            yaml
        );

        // Only the primary owns the certificate; the second just adds paths to a
        // host the first already serves.
        assert_eq!(yaml.matches("secretName: tls").count(), 1, "{}", yaml);

        let second = yaml.split("--limit-10485760").nth(1).unwrap();
        assert!(second.contains("      - path: /upload"), "{}", yaml);
        assert!(
            !second.contains(
                "      - path: /
"
            ),
            "{}",
            yaml
        );
    }

    #[test]
    fn rewrite_target_follows_the_group_that_needs_it() {
        let mut spec = spec_with_limits(&[("web", "/", None), ("upload", "/upload", Some(2048))]);
        spec.ingress.rules[0].services[1].strip_prefix = true;
        let yaml = render_ingress(&spec);

        // The annotation rewrites every path in its Ingress, so it must land on
        // the group that actually strips.
        let (first, second) = yaml.split_once("--limit-2048").unwrap();
        assert!(!first.contains("rewrite-target"), "{}", yaml);
        assert!(second.contains("rewrite-target"), "{}", yaml);
    }

    #[test]
    fn a_redirect_becomes_its_own_ingress_object() {
        let tls = IngressTlsResolvedSpec {
            secret: None,
            letsencrypt: Some(LetsEncryptResolvedSpec {
                server: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
                email: "ops@somesite.com".to_string(),
            }),
        };
        let yaml = render(&ingress(
            vec![redirect("somesite.com", "www.somesite.com", true)],
            Some(tls),
        ));

        assert!(
            yaml.starts_with(
                "---
"
            ),
            "{}",
            yaml
        );
        assert!(yaml.contains("  name: gateway--redirect-0"), "{}", yaml);
        assert!(
            yaml.contains("nginx.ingress.kubernetes.io/permanent-redirect: https://www.somesite.com"),
            "{}",
            yaml
        );
        assert!(yaml.contains("  - host: somesite.com"), "{}", yaml);
        // The redirect answers before the backend is reached, but a path still
        // has to name one, so it borrows a service that actually exists.
        assert!(yaml.contains("            name: api"), "{}", yaml);
        // Reuses the certificate the main Ingress owns, which already lists this
        // domain, instead of racing cert-manager for the same secret.
        assert!(yaml.contains("    secretName: gateway--tls"), "{}", yaml);
        assert!(!yaml.contains("cert-manager.io/cluster-issuer"), "{}", yaml);
    }

    #[test]
    fn sources_sharing_a_destination_share_one_ingress() {
        let yaml = render(&ingress(
            vec![
                redirect("somesite.com", "www.somesite.com", true),
                redirect("somesite.net", "www.somesite.com", true),
                redirect("old.somesite.com", "www.somesite.com", false),
            ],
            None,
        ));

        assert_eq!(yaml.matches("kind: Ingress").count(), 2, "{}", yaml);
        assert!(yaml.contains("  - host: somesite.com"), "{}", yaml);
        assert!(yaml.contains("  - host: somesite.net"), "{}", yaml);
        assert!(
            yaml.contains("nginx.ingress.kubernetes.io/temporal-redirect: http://www.somesite.com"),
            "{}",
            yaml
        );
        // No TLS configured, so no tls block to attach.
        assert!(!yaml.contains("secretName"), "{}", yaml);
    }

    #[test]
    fn a_redirect_without_any_routed_service_is_rejected() {
        let mut spec = ingress(vec![redirect("somesite.com", "www.somesite.com", true)], None);
        spec.rules.clear();

        let dir = tempfile::tempdir().unwrap();
        let mut file = File::create(dir.path().join("ingress.yaml")).unwrap();
        let err = generate_redirect_ingresses(&spec, &mut file).unwrap_err().to_string();
        assert!(err.contains("at least one service"), "unexpected error: {}", err);
    }

    fn workload(name: &str, service_type: ServiceType) -> ServiceResolvedSpec {
        ServiceResolvedSpec {
            service_type,
            is_app_service: true,
            full_name: name.to_string(),
            image: format!("registry.example.com/{}:1.0.0", name),
            environment_variables: vec![],
            undockerized_environment_variables: vec![],
            configs: vec![],
            secrets: vec![],
            ports: vec![crate::spec::ServicePort {
                external: 80,
                internal: 8080,
            }],
            expose: vec![],
            volumes: vec![],
            command: None,
            entrypoint: None,
            healthcheck: None,
            depends_on: vec![],
            resources: crate::spec::ResourcesSpec {
                replicas: 1,
                requests: crate::spec::ResourceLimits {
                    memory: "128Mi".to_string(),
                    cpu: "100m".to_string(),
                },
                limits: crate::spec::ResourceLimits {
                    memory: "256Mi".to_string(),
                    cpu: "200m".to_string(),
                },
            },
            working_dir: None,
        }
    }

    #[test]
    fn a_service_override_sets_its_own_replicas_and_resources() {
        let mut api = workload("api", ServiceType::Public);
        api.resources.replicas = 3;
        api.resources.limits.memory = "1Gi".to_string();
        let dir = generate_to_temp(vec![api]);

        let deployment = fs::read_to_string(dir.path().join("deployment-api.yaml")).unwrap();
        assert!(deployment.contains("  replicas: 3\n"), "{}", deployment);
        assert!(deployment.contains("            memory: 1Gi\n"), "{}", deployment);
        // The deployment defaults in the spec (1 replica, 1/1) are not what is written.
        assert!(!deployment.contains("  replicas: 1\n"), "{}", deployment);
    }

    fn generate_to_temp(services: Vec<ServiceResolvedSpec>) -> tempfile::TempDir {
        let mut spec = spec_with_limits(&[("api", "/", None)]);
        spec.current_deployment.services = services;
        let dir = tempfile::tempdir().unwrap();
        generate(&spec, dir.path()).unwrap();
        dir
    }

    #[test]
    fn names_are_normalized_to_dns_1123() {
        assert_eq!(k8s_name("shop-db_password"), "shop-db-password");
        assert_eq!(k8s_name("Main Svc"), "main-svc");
        assert_eq!(k8s_name("-api-"), "api");
        assert_eq!(k8s_name("api"), "api");
    }

    #[test]
    fn yaml_scalars_are_escaped() {
        assert_eq!(yaml_quote("plain"), "\"plain\"");
        assert_eq!(yaml_quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(yaml_quote("line1\nline2"), "\"line1\\nline2\"");
    }

    #[test]
    fn a_job_becomes_a_kubernetes_job_without_a_service() {
        let dir = generate_to_temp(vec![workload("migrate", ServiceType::Job)]);

        let job = fs::read_to_string(dir.path().join("job-migrate.yaml")).unwrap();
        assert!(job.contains("kind: Job"), "{}", job);
        assert!(job.contains("backoffLimit: 0"), "{}", job);
        assert!(job.contains("restartPolicy: Never"), "{}", job);
        // The name carries the deploy date, since a Job's template is immutable.
        assert!(job.contains("  name: migrate-"), "{}", job);
        assert!(!dir.path().join("deployment-migrate.yaml").exists());
        assert!(!dir.path().join("service-migrate.yaml").exists());
    }

    #[test]
    fn a_service_without_ports_gets_no_service_object() {
        let mut worker = workload("worker", ServiceType::Internal);
        worker.ports.clear();
        let dir = generate_to_temp(vec![worker]);

        assert!(dir.path().join("deployment-worker.yaml").exists());
        assert!(!dir.path().join("service-worker.yaml").exists());
    }

    #[test]
    fn underscored_secret_names_are_valid_everywhere_they_appear() {
        let mut api = workload("api", ServiceType::Public);
        api.secrets = vec![
            crate::spec::ServiceSecret {
                name: "shop-db_password".to_string(),
                mount: SecretMount::EnvVariable("DB_PASSWORD".to_string()),
            },
            crate::spec::ServiceSecret {
                name: "shop-tls_key".to_string(),
                mount: SecretMount::FilePath("/run/secrets/tls.key".to_string()),
            },
        ];
        let mut spec = spec_with_limits(&[("api", "/", None)]);
        spec.current_deployment.services = vec![api];
        spec.current_deployment.secrets = vec![
            crate::resolved_spec::SecretResolvedSpec {
                name: "shop-db_password".to_string(),
                value: crate::resolved_spec::SecretResolvedValue::Literal("pw".to_string()),
            },
            crate::resolved_spec::SecretResolvedSpec {
                name: "shop-tls_key".to_string(),
                value: crate::resolved_spec::SecretResolvedValue::Deferred(crate::spec::AwsSecretRef {
                    secret_id: "prod/tls".to_string(),
                    jq: None,
                }),
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        generate(&spec, dir.path()).unwrap();

        let secret = fs::read_to_string(dir.path().join("secret-shop-db-password.yaml")).unwrap();
        assert!(secret.contains("  name: shop-db-password"), "{}", secret);

        let deployment = fs::read_to_string(dir.path().join("deployment-api.yaml")).unwrap();
        assert!(
            deployment.contains("              name: shop-db-password"),
            "{}",
            deployment
        );
        assert!(
            deployment.contains("          secretName: shop-tls-key"),
            "{}",
            deployment
        );

        // The deferred secret is created under the same normalized name.
        let fetch = fs::read_to_string(dir.path().join("fetch-secrets.sh")).unwrap();
        assert!(
            fetch.contains("kubectl create secret generic 'shop-tls-key'"),
            "{}",
            fetch
        );
    }

    #[test]
    fn env_values_with_quotes_stay_valid_yaml() {
        let mut api = workload("api", ServiceType::Public);
        api.environment_variables = vec![crate::spec::EnvVariable {
            name: "GREETING".to_string(),
            value: "say \"hi\"\\now".to_string(),
        }];
        let dir = generate_to_temp(vec![api]);

        let deployment = fs::read_to_string(dir.path().join("deployment-api.yaml")).unwrap();
        assert!(
            deployment.contains("value: \"say \\\"hi\\\"\\\\now\""),
            "{}",
            deployment
        );
        let parsed: serde_yaml::Value = serde_yaml::from_str(&deployment).unwrap();
        let env = &parsed["spec"]["template"]["spec"]["containers"][0]["env"][1];
        assert_eq!(env["value"].as_str(), Some("say \"hi\"\\now"));
    }
}
