use crate::resolved_spec::{EnvironmentResolvedSpec, LetsEncryptResolvedSpec};
use crate::spec::{parse_duration_secs, Healthcheck, SecretMount};
use anyhow::Result;
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

    // 2. Secrets
    for secret in &deployment.secrets {
        let file_name = output_dir.join(format!("secret-{}.yaml", secret.name));
        let mut file = File::create(file_name)?;
        writeln!(file, "apiVersion: v1")?;
        writeln!(file, "kind: Secret")?;
        writeln!(file, "metadata:")?;
        writeln!(file, "  name: {}", secret.name)?;
        writeln!(file, "type: Opaque")?;
        writeln!(file, "data:")?;
        let encoded = general_purpose::STANDARD.encode(&secret.value);
        writeln!(file, "  {}: {}", "value", encoded)?; 
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
        for domain in &resolved_spec.ingress.domains {
            writeln!(file, "    - {}", domain)?;
        }
        if let Some(secret) = &tls.secret {
             writeln!(file, "    secretName: {}", secret)?;
        } else if tls.letsencrypt.is_some() {
             writeln!(file, "    secretName: {}--tls", resolved_spec.ingress.name)?;
        }
    }
    
    writeln!(file, "  rules:")?;

    for rule in &resolved_spec.ingress.rules {
        writeln!(file, "  - host: {}", rule.domain_name)?;
        writeln!(file, "    http:")?;
        writeln!(file, "      paths:")?;
        
        for svc_rule in &rule.services {
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
