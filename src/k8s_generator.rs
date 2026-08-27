use crate::resolved_spec::{EnvironmentResolvedSpec, IngressResolvedSpec, LetsEncryptResolvedSpec};
use crate::secret_fetch::{self, sh_quote, FetchScript};
use crate::spec::{parse_duration_secs, Healthcheck, SecretMount};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use base64::{Engine as _, engine::general_purpose};

const LETSENCRYPT_ISSUER: &str = "letsencrypt-prod";

pub fn generate(
    resolved_spec: &EnvironmentResolvedSpec,
    output_dir: &Path,
) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }

    let deployment = &resolved_spec.current_deployment;

    // 1. ConfigMaps
    for config in &deployment.configs {
        let file_name = output_dir.join(format!("configmap-{}.yaml", config.name));
        let mut file = File::create(file_name)?;
        writeln!(file, "apiVersion: v1")?;
        writeln!(file, "kind: ConfigMap")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", config.name)?;
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
            fetch_script.command(format!(
                "kubectl create secret generic {name} --from-literal=value=\"${var}\" \
                 --dry-run=client -o yaml | kubectl apply -f -",
                name = sh_quote(&secret.name),
                var = secret.shell_var(),
            ));
            continue;
        }

        let file_name = output_dir.join(format!("secret-{}.yaml", secret.name));
        let mut file = File::create(file_name)?;
        writeln!(file, "apiVersion: v1")?;
        writeln!(file, "kind: Secret")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", secret.name)?;
        writeln!(file, "type: Opaque")?;
        writeln!(file, "data:")?;
        let encoded = general_purpose::STANDARD.encode(secret.literal().unwrap_or_default());
        writeln!(file, "  {}: {}", "value", encoded)?;
    }

    if !fetch_script.is_empty() {
        fetch_script.write(
            &output_dir.join(secret_fetch::SCRIPT_NAME),
            "Run it against the target cluster before `kubectl apply -f .`; it creates the \
             Secrets the manifests here reference.",
        )?;
    }

    // 3. Deployments & Services
    for service in &deployment.services {
        let file_name = output_dir.join(format!("deployment-{}.yaml", service.full_name));
        let mut file = File::create(file_name)?;
        
        // Deployment
        writeln!(file, "apiVersion: apps/v1")?;
        writeln!(file, "kind: Deployment")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", service.full_name)?;
        writeln!(file, "spec:")?;
        writeln!(file, "  replicas: {}", deployment.defaults.replicas)?; 
        writeln!(file, "  selector:")?;
        writeln!(file, "    matchLabels:")?;
        writeln!(file, "      app: {}", service.full_name)?;
        writeln!(file, "  template:")?;
        writeln!(file, "    metadata:")?;
        writeln!(file, "      labels:")?;
        writeln!(file, "        app: {}", service.full_name)?;
        writeln!(file, "    spec:")?;
        writeln!(file, "      containers:")?;
        writeln!(file, "      - name: {}", service.full_name)?;
        writeln!(file, "        image: {}", service.image)?;
        // docker-compose `entrypoint` overrides the image ENTRYPOINT, which maps
        // to a container's `command` in Kubernetes; `command` overrides the image
        // CMD, which maps to a container's `args`.
        if let Some(entrypoint) = &service.entrypoint {
            writeln!(file, "        command:")?;
            for arg in entrypoint.to_args() {
                writeln!(file, "        - \"{}\"", arg)?;
            }
        }
        if let Some(command) = &service.command {
            writeln!(file, "        args:")?;
            for arg in command.to_args() {
                writeln!(file, "        - \"{}\"", arg)?;
            }
        }
        // docker-compose `healthcheck` maps to liveness/readiness probes.
        if let Some(hc) = &service.healthcheck {
            if let Some(argv) = hc.probe_argv() {
                write_probe(&mut file, "livenessProbe", &argv, hc)?;
                write_probe(&mut file, "readinessProbe", &argv, hc)?;
            }
        }
        writeln!(file, "        resources:")?;
        writeln!(file, "          requests:")?;
        writeln!(file, "            memory: {}", deployment.defaults.requests.memory)?;
        writeln!(file, "            cpu: {}", deployment.defaults.requests.cpu)?;
        writeln!(file, "          limits:")?;
        writeln!(file, "            memory: {}", deployment.defaults.limits.memory)?;
        writeln!(file, "            cpu: {}", deployment.defaults.limits.cpu)?;

        let deploy_date = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writeln!(file, "        env:")?;
        writeln!(file, "        - name: DEPLOY_DATE")?;
        writeln!(file, "          value: \"{}\"", deploy_date)?;
        for env in &service.environment_variables {
            writeln!(file, "        - name: {}", env.name)?;
            writeln!(file, "          value: \"{}\"", env.value)?;
        }
        for secret in &service.secrets {
             if let SecretMount::EnvVariable(var_name) = &secret.mount {
                 writeln!(file, "        - name: {}", var_name)?;
                 writeln!(file, "          valueFrom:")?;
                 writeln!(file, "            secretKeyRef:")?;
                 writeln!(file, "              name: {}", secret.name)?;
                 writeln!(file, "              key: value")?;
             }
        }
        
        // Volume Mounts (Configs & Secrets)
        let has_volume_mounts = !service.configs.is_empty() || service.secrets.iter().any(|s| !matches!(s.mount, SecretMount::EnvVariable(_)));
        
        if has_volume_mounts {
             writeln!(file, "        volumeMounts:")?;
             for config in &service.configs {
                 writeln!(file, "        - name: config-{}", config.config_name)?;
                 writeln!(file, "          mountPath: {}", config.mount_path)?;
             }
             for secret in &service.secrets {
                 match &secret.mount {
                     SecretMount::FilePath(path) => {
                         writeln!(file, "        - name: secret-{}", secret.name)?;
                         writeln!(file, "          mountPath: {}", path)?;
                     }
                     SecretMount::EnvVariable(_) => {}
                 }
             }
        }
        
        if has_volume_mounts {
            writeln!(file, "      volumes:")?;
            for config in &service.configs {
                 writeln!(file, "      - name: config-{}", config.config_name)?;
                 writeln!(file, "        configMap:")?;
                 writeln!(file, "          name: {}", config.config_name)?;
            }
            for secret in &service.secrets {
                 if !matches!(secret.mount, SecretMount::EnvVariable(_)) {
                     writeln!(file, "      - name: secret-{}", secret.name)?;
                     writeln!(file, "        secret:")?;
                     writeln!(file, "          secretName: {}", secret.name)?;
                 }
            }
        }
        
        // Service
        let svc_file_name = output_dir.join(format!("service-{}.yaml", service.full_name));
        let mut svc_file = File::create(svc_file_name)?;
        writeln!(svc_file, "apiVersion: v1")?;
        writeln!(svc_file, "kind: Service")?;
        writeln!(svc_file, "metadata:")?;
        writeln!(svc_file, "  name: {}", service.full_name)?;
        writeln!(svc_file, "spec:")?;
        writeln!(svc_file, "  selector:")?;
        writeln!(svc_file, "    app: {}", service.full_name)?;
        writeln!(svc_file, "  ports:")?;
        for port in &service.ports {
            writeln!(svc_file, "  - port: {}", port.external)?;
            writeln!(svc_file, "    targetPort: {}", port.internal)?;
        }
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
        writeln!(file, "            - \"{}\"", arg)?;
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

fn generate_ingress(
    resolved_spec: &EnvironmentResolvedSpec,
    output_dir: &Path
) -> Result<()> {
    let file_name = output_dir.join("ingress.yaml");
    let mut file = File::create(file_name)?;
    
    writeln!(file, "apiVersion: networking.k8s.io/v1")?;
    writeln!(file, "kind: Ingress")?;
    writeln!(file, "metadata:")?;
    writeln!(file, "  name: {}", resolved_spec.ingress.name)?;
    writeln!(file, "  annotations:")?;
    // Annotations for strip-prefix and cert-manager
    if let Some(tls) = &resolved_spec.ingress.tls {
        if tls.letsencrypt.is_some() {
            writeln!(file, "    cert-manager.io/cluster-issuer: {}", LETSENCRYPT_ISSUER)?;
        }
    }
    // Check if any rule needs strip-prefix
    let needs_strip_prefix = resolved_spec.ingress.rules.iter().any(|r| r.services.iter().any(|s| s.strip_prefix));
    if needs_strip_prefix {
        writeln!(file, "    nginx.ingress.kubernetes.io/rewrite-target: /$2")?;
    }

    writeln!(file, "spec:")?;
    writeln!(file, "  ingressClassName: nginx")?;
    if let Some(tls) = &resolved_spec.ingress.tls {
        writeln!(file, "  tls:")?;
        writeln!(file, "  - hosts:")?;
        // The same domain can be declared under multiple host groups, so
        // `ingress.domains` may contain duplicates; emit each host only once.
        let mut seen_hosts: Vec<&String> = Vec::new();
        for domain in &resolved_spec.ingress.domains {
            if seen_hosts.contains(&domain) {
                continue;
            }
            seen_hosts.push(domain);
            writeln!(file, "    - {}", domain)?;
        }
        if let Some(secret) = &tls.secret {
             writeln!(file, "    secretName: {}", secret)?;
        } else if tls.letsencrypt.is_some() {
             writeln!(file, "    secretName: {}--tls", resolved_spec.ingress.name)?;
        }
    }
    
    writeln!(file, "  rules:")?;

    // The same domain can be declared under multiple host groups, producing
    // several rules with the same domain_name. Emit one `- host:` entry per
    // domain with all of its services merged, rather than repeating the host.
    // `domains` preserves first-seen order.
    let mut domains: Vec<&String> = Vec::new();
    let mut services_by_domain: std::collections::HashMap<
        &String,
        Vec<&crate::resolved_spec::IngressToServiceRule>,
    > = std::collections::HashMap::new();
    for rule in &resolved_spec.ingress.rules {
        if !services_by_domain.contains_key(&rule.domain_name) {
            domains.push(&rule.domain_name);
        }
        services_by_domain
            .entry(&rule.domain_name)
            .or_default()
            .extend(rule.services.iter());
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
             
             let path_type = if svc_rule.strip_prefix { "ImplementationSpecific" } else { "Prefix" };
             writeln!(file, "      - path: {}", path)?;
             writeln!(file, "        pathType: {}", path_type)?;
             writeln!(file, "        backend:")?;
             writeln!(file, "          service:")?;
             writeln!(file, "            name: {}", svc_rule.service_name)?;
             writeln!(file, "            port:")?;
             writeln!(file, "              number: {}", svc_rule.port)?;
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
        tls.secret.clone()
            .or_else(|| tls.letsencrypt.as_ref().map(|_| format!("{}--tls", ingress.name)))
    });

    for (index, key) in targets.iter().enumerate() {
        let (to, permanent) = *key;
        let sources = &sources_by_target[key];
        let target_url = ingress.redirects.iter()
            .find(|r| r.to == to && r.permanent == permanent)
            .map(|r| r.target_url(has_tls))
            .unwrap_or_else(|| to.to_string());

        writeln!(file, "---")?;
        writeln!(file, "apiVersion: networking.k8s.io/v1")?;
        writeln!(file, "kind: Ingress")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}--redirect-{}", ingress.name, index)?;
        writeln!(file, "  annotations:")?;
        let annotation = if permanent { "permanent-redirect" } else { "temporal-redirect" };
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
            writeln!(file, "            name: {}", placeholder.service_name)?;
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
    use crate::resolved_spec::{IngressRule, IngressToServiceRule, IngressTlsResolvedSpec, RedirectRule};
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

    #[test]
    fn a_redirect_becomes_its_own_ingress_object() {
        let tls = IngressTlsResolvedSpec {
            secret: None,
            letsencrypt: Some(LetsEncryptResolvedSpec {
                server: "https://acme-v02.api.letsencrypt.org/directory".to_string(),
                email: "ops@somesite.com".to_string(),
            }),
        };
        let yaml = render(&ingress(vec![redirect("somesite.com", "www.somesite.com", true)], Some(tls)));

        assert!(yaml.starts_with("---
"), "{}", yaml);
        assert!(yaml.contains("  name: gateway--redirect-0"), "{}", yaml);
        assert!(yaml.contains("nginx.ingress.kubernetes.io/permanent-redirect: https://www.somesite.com"), "{}", yaml);
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
        assert!(yaml.contains("nginx.ingress.kubernetes.io/temporal-redirect: http://www.somesite.com"), "{}", yaml);
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
}
