use crate::resolved_spec::{EnvironmentResolvedSpec, IngressResolvedSpec};
use crate::spec::{DockerIngressType, DockerSpecificSpec, SecretMount};
use anyhow::{anyhow, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

pub fn generate(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }
    
    if docker_spec.swarm_mode {
        generate_swarm(resolved_spec, docker_spec, output_dir)
    } else {
        generate_standalone(resolved_spec, docker_spec, output_dir)
    }
}

fn generate_standalone(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    let deployment = &resolved_spec.current_deployment;
    let app_name = &deployment.application_name;
    
    // Create subdirs
    let configs_dir = output_dir.join("configs");
    fs::create_dir_all(&configs_dir)?;
    
    // 1. Configs
    for config in &deployment.configs {
         let dir_name = format!("{}-{}", app_name, config.name);
         let cfg_dir = configs_dir.join(&dir_name);
         fs::create_dir_all(&cfg_dir)?;
         for cfg_file in &config.files {
             let path = cfg_dir.join(&cfg_file.name);
             fs::write(&path, &cfg_file.content)?;
         }
    }
    
    // 2. Secrets
    let secrets_dir = output_dir.join("secrets");
    fs::create_dir_all(&secrets_dir)?;
    for secret in &deployment.secrets {
        let secret_name = format!("{}-{}", app_name, secret.name);
        let path = secrets_dir.join(&secret_name);
        fs::write(&path, &secret.value)?;
    }

    // 3. Envs
    let envs_dir = output_dir.join("envs");
    fs::create_dir_all(&envs_dir)?;

    // 4. Script
    let mut deploy_sh = File::create(output_dir.join("deploy.sh"))?;
    
    #[cfg(unix)]
    {
        let mut perms = deploy_sh.metadata()?.permissions();
        perms.set_mode(0o755);
        deploy_sh.set_permissions(perms)?;
    }

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;
    
    let network_name = format!("{}-net", resolved_spec.name);
    writeln!(deploy_sh, "docker network create {} || true", network_name)?;
    
    for service in &deployment.services {
         writeln!(deploy_sh, "echo 'Starting {}...'", service.full_name)?;
         writeln!(deploy_sh, "docker rm -f {} || true", service.full_name)?;
         
         // Create env file
         let env_file_name = format!("{}.env", service.full_name);
         let env_path = envs_dir.join(&env_file_name);
         let mut env_file = File::create(&env_path)?;
         
         for env in &service.environment_variables {
             writeln!(env_file, "{}={}", env.name, env.value)?;
         }

         write!(deploy_sh, "docker run -d --name {} --network {}", service.full_name, network_name)?;
         
         for port in &service.ports {
             write!(deploy_sh, " -p {}:{}", port.external, port.internal)?;
         }
         
         write!(deploy_sh, " --env-file $(pwd)/envs/{}", env_file_name)?;
         
         for secret in &service.secrets {
              if let SecretMount::EnvVariable(var_name) = &secret.mount {
                   write!(deploy_sh, " -e {}=$(cat ./secrets/{})", var_name, secret.name)?;
              }
         }
         
         for config in &service.configs {
              let local_path = format!("$(pwd)/configs/{}", config.config_name);
              write!(deploy_sh, " -v {}:{}", local_path, config.mount_path)?;
         }
         
         for secret in &service.secrets {
             if let SecretMount::FilePath(path) = &secret.mount {
                  let local_path = format!("$(pwd)/secrets/{}", secret.name);
                  write!(deploy_sh, " -v {}:{}", local_path, path)?;
             }
         }
         
         writeln!(deploy_sh, " {}", service.image)?;
    }
    
    // Ingress Container
    if !resolved_spec.ingress.rules.is_empty() {
        match docker_spec.ingress_type {
            DockerIngressType::Nginx => generate_nginx_standalone(resolved_spec, output_dir, &mut deploy_sh, network_name)?,

            DockerIngressType::Traefik => {
                generate_traefik_standalone(resolved_spec, output_dir, &mut deploy_sh, network_name)?;
            }
        }

    }
    
    Ok(())
}

fn generate_swarm(
    resolved_spec: &EnvironmentResolvedSpec,
    docker_spec: &DockerSpecificSpec,
    output_dir: &Path,
) -> Result<()> {
    let deployment = &resolved_spec.current_deployment;
    let app_name = &deployment.application_name;
    
    // Application folder
    let app_dir = output_dir.join(&deployment.name);
    fs::create_dir_all(&app_dir)?;

    // 1. Configs
    let configs_dir = app_dir.join("configs");
    fs::create_dir_all(&configs_dir)?;
    
    for config in &deployment.configs {
         let dir_name = format!("{}-{}", app_name, config.name);
         let cfg_dir = configs_dir.join(&dir_name);
         fs::create_dir_all(&cfg_dir)?;
         for cfg_file in &config.files {
             let path = cfg_dir.join(&cfg_file.name);
             fs::write(&path, &cfg_file.content)?;
         }
    }
    
    // 2. Secrets
    let secrets_dir = app_dir.join("secrets");
    fs::create_dir_all(&secrets_dir)?;
    for secret in &deployment.secrets {
        let secret_name = format!("{}-{}", app_name, secret.name);
        let path = secrets_dir.join(&secret_name);
        fs::write(&path, &secret.value)?;
    }

    // 3. Application Stack
    let mut app_stack = File::create(app_dir.join("docker-compose.yml"))?;
    writeln!(app_stack, "version: '3.8'")?;
    writeln!(app_stack, "services:")?;
    
    let network_name = format!("{}-net", resolved_spec.name);

    for service in &deployment.services {
        writeln!(app_stack, "  {}:", service.full_name)?;
        writeln!(app_stack, "    image: {}", service.image)?;
        writeln!(app_stack, "    networks:")?;
        writeln!(app_stack, "      default:")?;
        writeln!(app_stack, "        aliases:")?;
        writeln!(app_stack, "          - {}", service.full_name)?;
        
        if !service.ports.is_empty() {
            writeln!(app_stack, "    ports:")?;
            for port in &service.ports {
                writeln!(app_stack, "      - \"{}:{}\"", port.external, port.internal)?;
            }
        }

        let mut has_env = !service.environment_variables.is_empty();
        // Check if we have env secrets
        for secret in &service.secrets {
            if let SecretMount::EnvVariable(_) = &secret.mount {
                has_env = true;
                break;
            }
        }

        if has_env {
            writeln!(app_stack, "    environment:")?;
            for env in &service.environment_variables {
                writeln!(app_stack, "      - {}={}", env.name, env.value)?;
            }
            // Inject secrets as env vars
            for secret in &service.secrets {
                 if let SecretMount::EnvVariable(var_name) = &secret.mount {
                      if let Some(s_spec) = deployment.secrets.iter().find(|s| s.name == secret.name) {
                          // Note: secret.name in ServiceSecret is "app-name-secretname"
                          // deployment.secrets has "app-name-secretname"? 
                          // In resolved_spec: "secret name transformed to app name + original name"
                          // So yes, they should match.
                          writeln!(app_stack, "      - {}={}", var_name, s_spec.value)?;
                      }
                 }
            }
        }

        let mut volumes = Vec::new();
        for config in &service.configs {
             // Mount directory
             volumes.push(format!("./configs/{}:{}", config.config_name, config.mount_path));
        }
        for secret in &service.secrets {
             if let SecretMount::FilePath(path) = &secret.mount {
                  volumes.push(format!("./secrets/{}:{}", secret.name, path));
             }
        }

        if !volumes.is_empty() {
            writeln!(app_stack, "    volumes:")?;
            for vol in volumes {
                writeln!(app_stack, "      - \"{}\"", vol)?;
            }
        }
    }

    writeln!(app_stack, "networks:")?;
    writeln!(app_stack, "  default:")?;
    writeln!(app_stack, "    external: true")?;
    writeln!(app_stack, "    name: {}", network_name)?;

    // 4. Ingress Stack
    let ingress_dir = output_dir.join("ingress");
    fs::create_dir_all(&ingress_dir)?;
    
    match docker_spec.ingress_type {
        DockerIngressType::Nginx => generate_nginx_swarm(resolved_spec, &ingress_dir, network_name.clone())?,
        DockerIngressType::Traefik => generate_traefik_swarm(resolved_spec, &ingress_dir, network_name.clone())?,
    }

    // 5. Deploy Script
    let mut deploy_sh = File::create(output_dir.join("deploy.sh"))?;
    
    #[cfg(unix)]
    {
        let mut perms = deploy_sh.metadata()?.permissions();
        perms.set_mode(0o755);
        deploy_sh.set_permissions(perms)?;
    }

    writeln!(deploy_sh, "#!/bin/bash")?;
    writeln!(deploy_sh, "set -e")?;
    writeln!(deploy_sh, "docker network create --driver overlay --attachable {} || true", network_name)?;
    writeln!(deploy_sh, "docker stack deploy -c ingress/docker-compose.yml ingress")?;
    writeln!(deploy_sh, "docker stack deploy -c {}/docker-compose.yml {}", deployment.name, deployment.name)?;

    Ok(())
}

fn generate_nginx_standalone(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path, deploy_sh: &mut File, network_name: String) -> Result<()> {
    if !resolved_spec.ingress.rules.is_empty() {
        let nginx_dir = output_dir.join("nginx");
        fs::create_dir_all(&nginx_dir)?;
        generate_nginx_config(&resolved_spec.ingress, &nginx_dir.join("default.conf"))?;
    }

    writeln!(deploy_sh, "echo 'Starting Nginx ingress...'")?;
    writeln!(deploy_sh, "docker rm -f nginx-ingress || true")?;
    write!(deploy_sh, "docker run -d --name nginx-ingress --network {}", network_name)?;
    write!(deploy_sh, " -p 80:80")?;

    let has_tls = resolved_spec.ingress.tls.is_some();
    if has_tls {
        write!(deploy_sh, " -p 443:443")?;
    }

    write!(deploy_sh, " -v $(pwd)/nginx/default.conf:/etc/nginx/conf.d/default.conf")?;

    if has_tls {
        fs::create_dir_all(output_dir.join("certs"))?;
        write!(deploy_sh, " -v $(pwd)/certs:/etc/nginx/certs")?;
    }

    if let Some(tls) = &resolved_spec.ingress.tls {
        if tls.letsencrypt.is_some() {
            write!(deploy_sh, " -v $(pwd)/letsencrypt:/var/www/letsencrypt")?;
            fs::create_dir_all(output_dir.join("letsencrypt"))?;
        }
    }

    writeln!(deploy_sh, " nginx:alpine")?;

    if let Some(tls) = &resolved_spec.ingress.tls {
        if let Some(le) = &tls.letsencrypt {
            let mut certbot_sh = File::create(output_dir.join("certbot.sh"))?;
            writeln!(certbot_sh, "docker run -it --rm --name certbot \\")?;
            writeln!(certbot_sh, "  -v $(pwd)/letsencrypt:/var/www/letsencrypt \\")?;
            writeln!(certbot_sh, "  -v $(pwd)/certs:/etc/nginx/certs \\")?;
            writeln!(certbot_sh, "  certbot/certbot certonly --webroot --webroot-path=/var/www/letsencrypt \\")?;
            writeln!(certbot_sh, "  --email {} --agree-tos --no-eff-email \\", le.email)?;
            for domain in &resolved_spec.ingress.domains {
                writeln!(certbot_sh, "   -d {} \\", domain)?;
            }
            writeln!(certbot_sh, "   && docker restart nginx-ingress")?;
        }
    }
    Ok(())
}

fn generate_nginx_swarm(resolved_spec: &EnvironmentResolvedSpec, ingress_dir: &Path, network_name: String) -> Result<()> {
    if resolved_spec.ingress.rules.is_empty() {
        return Ok(());
    }

    let nginx_conf_dir = ingress_dir.join("nginx");
    fs::create_dir_all(&nginx_conf_dir)?;
    generate_nginx_config(&resolved_spec.ingress, &nginx_conf_dir.join("default.conf"))?;
    
    let mut stack = File::create(ingress_dir.join("docker-compose.yml"))?;
    writeln!(stack, "version: '3.8'")?;
    writeln!(stack, "services:")?;
    writeln!(stack, "  nginx:")?;
    writeln!(stack, "    image: nginx:alpine")?;
    writeln!(stack, "    ports:")?;
    writeln!(stack, "      - \"80:80\"")?;
    if resolved_spec.ingress.tls.is_some() {
        writeln!(stack, "      - \"443:443\"")?;
    }
    writeln!(stack, "    volumes:")?;
    writeln!(stack, "      - ./nginx/default.conf:/etc/nginx/conf.d/default.conf")?;
    
    if resolved_spec.ingress.tls.is_some() {
         // We assume certs are placed in output_dir/certs -> so from ingress/docker-compose.yml, it is ../certs
         // Wait, the structure is output_dir/ingress/docker-compose.yml
         // So ../certs is output_dir/certs
         writeln!(stack, "      - ../certs:/etc/nginx/certs")?;
         
         if let Some(tls) = &resolved_spec.ingress.tls {
            if tls.letsencrypt.is_some() {
                 writeln!(stack, "      - ../letsencrypt:/var/www/letsencrypt")?;
            }
         }
    }
    
    writeln!(stack, "    networks:")?;
    writeln!(stack, "      default:")?;
    writeln!(stack, "networks:")?;
    writeln!(stack, "  default:")?;
    writeln!(stack, "    external: true")?;
    writeln!(stack, "    name: {}", network_name)?;
    
    Ok(())
}

fn generate_nginx_config(ingress: &IngressResolvedSpec, path: &Path) -> Result<()> {
    let mut file = File::create(path)?;
    
    let has_tls = ingress.tls.is_some();
    
    for rule in &ingress.rules {
        writeln!(file, "server {{")?;
        writeln!(file, "    listen 80;")?;
        writeln!(file, "    server_name {};", rule.domain_name)?;
        
        if let Some(tls) = &ingress.tls {
            if tls.letsencrypt.is_some() {
                writeln!(file, "    location /.well-known/acme-challenge/ {{")?;
                writeln!(file, "        root /var/www/letsencrypt;")?;
                writeln!(file, "    }}")?;
            }
        }
        
        if has_tls {
            writeln!(file, "    location / {{")?;
            writeln!(file, "        return 301 https://$host$request_uri;")?;
            writeln!(file, "    }}")?;
            writeln!(file, "}}")?;
            
            writeln!(file, "server {{")?;
            writeln!(file, "    listen 443 ssl;")?;
            writeln!(file, "    server_name {};", rule.domain_name)?;
            writeln!(file, "    ssl_certificate /etc/nginx/certs/live/{}/fullchain.pem;", rule.domain_name)?;
            writeln!(file, "    ssl_certificate_key /etc/nginx/certs/live/{}/privkey.pem;", rule.domain_name)?;
            
            generate_locations(&mut file, rule)?;
            
            writeln!(file, "}}")?;
            
        } else {
            generate_locations(&mut file, rule)?;
            writeln!(file, "}}")?;
        }
    }
    
    Ok(())
}

fn generate_locations(file: &mut File, rule: &crate::resolved_spec::IngressRule) -> Result<()> {
    for svc in &rule.services {
        let prefix = &svc.prefix;
        let location_path = if prefix.ends_with('/') {
            prefix.clone()
        } else {
            format!("{}/", prefix)
        };
        
        writeln!(file, "    location {} {{", location_path)?;
        
        if svc.strip_prefix {
            writeln!(file, "        proxy_pass http://{}:{}/;", svc.service_name, svc.port)?;
        } else {
            writeln!(file, "        proxy_pass http://{}:{};", svc.service_name, svc.port)?;
        }
        
        writeln!(file, "        proxy_set_header Host $host;")?;
        writeln!(file, "        proxy_set_header X-Real-IP $remote_addr;")?;
        writeln!(file, "        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;")?;
        writeln!(file, "        proxy_set_header X-Forwarded-Proto $scheme;")?;
        writeln!(file, "    }}")?;
    }
    Ok(())
}

fn generate_traefik_standalone(resolved_spec: &EnvironmentResolvedSpec, output_dir: &Path, deploy_sh: &mut File, network_name: String) -> Result<()> {
    let traefik_dir = output_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    let has_tls = resolved_spec.ingress.tls.is_some();
    let letsencrypt = resolved_spec.ingress.tls.as_ref().and_then(|t| t.letsencrypt.as_ref());

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    writeln!(static_conf, "entryPoints:")?;
    writeln!(static_conf, "  web:")?;
    writeln!(static_conf, "    address: \":80\"")?;
    if has_tls {
        writeln!(static_conf, "    http:")?;
        writeln!(static_conf, "      redirections:")?;
        writeln!(static_conf, "        entryPoint:")?;
        writeln!(static_conf, "          to: websecure")?;
        writeln!(static_conf, "          scheme: https")?;
        
        writeln!(static_conf, "  websecure:")?;
        writeln!(static_conf, "    address: \":443\"")?;
    }

    writeln!(static_conf, "providers:")?;
    writeln!(static_conf, "  file:")?;
    writeln!(static_conf, "    filename: \"/etc/traefik/dynamic_conf.yml\"")?;
    writeln!(static_conf, "    watch: true")?;

    if let Some(le) = letsencrypt {
        writeln!(static_conf, "certificatesResolvers:")?;
        writeln!(static_conf, "  myresolver:")?;
        writeln!(static_conf, "    acme:")?;
        writeln!(static_conf, "      email: \"{}\"", le.email)?;
        writeln!(static_conf, "      storage: \"/letsencrypt/acme.json\"")?;
        writeln!(static_conf, "      httpChallenge:")?;
        writeln!(static_conf, "        entryPoint: web")?;
    }

    generate_traefik_dynamic_config(&resolved_spec.ingress, &traefik_dir.join("dynamic_conf.yml"))?;

    writeln!(deploy_sh, "echo 'Starting Traefik ingress...'")?;
    writeln!(deploy_sh, "docker rm -f traefik-ingress || true")?;
    
    write!(deploy_sh, "docker run -d --name traefik-ingress --network {}", network_name)?;
    write!(deploy_sh, " -p 80:80")?;
    if has_tls {
        write!(deploy_sh, " -p 443:443")?;
    }

    write!(deploy_sh, " -v $(pwd)/traefik/traefik.yml:/etc/traefik/traefik.yml")?;
    write!(deploy_sh, " -v $(pwd)/traefik/dynamic_conf.yml:/etc/traefik/dynamic_conf.yml")?;

    if letsencrypt.is_some() {
        let le_dir = output_dir.join("letsencrypt");
        fs::create_dir_all(&le_dir)?;
        write!(deploy_sh, " -v $(pwd)/letsencrypt:/letsencrypt")?;
    }

    writeln!(deploy_sh, " traefik:v2.10")?;

    Ok(())
}

fn generate_traefik_swarm(resolved_spec: &EnvironmentResolvedSpec, ingress_dir: &Path, network_name: String) -> Result<()> {
    let traefik_dir = ingress_dir.join("traefik");
    fs::create_dir_all(&traefik_dir)?;

    // Reuse generate logic for config, but write to new dir
    let has_tls = resolved_spec.ingress.tls.is_some();
    let letsencrypt = resolved_spec.ingress.tls.as_ref().and_then(|t| t.letsencrypt.as_ref());

    let mut static_conf = File::create(traefik_dir.join("traefik.yml"))?;
    writeln!(static_conf, "entryPoints:")?;
    writeln!(static_conf, "  web:")?;
    writeln!(static_conf, "    address: \":80\"")?;
    if has_tls {
        writeln!(static_conf, "    http:")?;
        writeln!(static_conf, "      redirections:")?;
        writeln!(static_conf, "        entryPoint:")?;
        writeln!(static_conf, "          to: websecure")?;
        writeln!(static_conf, "          scheme: https")?;
        
        writeln!(static_conf, "  websecure:")?;
        writeln!(static_conf, "    address: \":443\"")?;
    }

    writeln!(static_conf, "providers:")?;
    writeln!(static_conf, "  file:")?;
    writeln!(static_conf, "    filename: \"/etc/traefik/dynamic_conf.yml\"")?;
    writeln!(static_conf, "    watch: true")?;

    if let Some(le) = letsencrypt {
        writeln!(static_conf, "certificatesResolvers:")?;
        writeln!(static_conf, "  myresolver:")?;
        writeln!(static_conf, "    acme:")?;
        writeln!(static_conf, "      email: \"{}\"", le.email)?;
        writeln!(static_conf, "      storage: \"/letsencrypt/acme.json\"")?;
        writeln!(static_conf, "      httpChallenge:")?;
        writeln!(static_conf, "        entryPoint: web")?;
    } else {
        return Err(anyhow!("Currently swarm ingress only supports Let's Encrypt, specify a letsencrypt block in ingress.tls"));
    }

    generate_traefik_dynamic_config(&resolved_spec.ingress, &traefik_dir.join("dynamic_conf.yml"))?;

    let mut stack = File::create(ingress_dir.join("docker-compose.yml"))?;
    writeln!(stack, "version: '3.8'")?;
    writeln!(stack, "services:")?;
    writeln!(stack, "  traefik:")?;
    writeln!(stack, "    image: traefik:v2.10")?;
    writeln!(stack, "    ports:")?;
    writeln!(stack, "      - \"80:80\"")?;
    if has_tls {
        writeln!(stack, "      - \"443:443\"")?;
    }
    writeln!(stack, "    volumes:")?;
    writeln!(stack, "      - ./traefik/traefik.yml:/etc/traefik/traefik.yml")?;
    writeln!(stack, "      - ./traefik/dynamic_conf.yml:/etc/traefik/dynamic_conf.yml")?;
    
    // Mount letsencrypt if needed. Using ../letsencrypt as in nginx
    if letsencrypt.is_some() {
        writeln!(stack, "      - ../letsencrypt:/letsencrypt")?;
        // Make sure dir exists
        fs::create_dir_all(ingress_dir.parent().unwrap().join("letsencrypt"))?;
    }
    
    writeln!(stack, "    networks:")?;
    writeln!(stack, "      default:")?;
    writeln!(stack, "networks:")?;
    writeln!(stack, "  default:")?;
    writeln!(stack, "    external: true")?;
    writeln!(stack, "    name: {}", network_name)?;
    
    Ok(())
}

fn generate_traefik_dynamic_config(ingress: &IngressResolvedSpec, path: &Path) -> Result<()> {
    let mut file = File::create(path)?;
    let has_tls = ingress.tls.is_some();
    let use_le = ingress.tls.as_ref().map(|t| t.letsencrypt.is_some()).unwrap_or(false);

    writeln!(file, "http:")?;
    
    let mut middlewares_written = false;
     for rule in ingress.rules.iter() {
         let router_name_base = rule.domain_name.replace(".", "-");
         for (j, svc) in rule.services.iter().enumerate() {
             if svc.strip_prefix && svc.prefix != "/" {
                 if !middlewares_written {
                     writeln!(file, "  middlewares:")?;
                     middlewares_written = true;
                 }
                 writeln!(file, "    strip-{}-{}:", router_name_base, j)?;
                 writeln!(file, "      stripPrefix:")?;
                 writeln!(file, "        prefixes:")?;
                 writeln!(file, "          - \"{}\"", svc.prefix)?;
             }
         }
    }
    
    writeln!(file, "  routers:")?;
    for (i, rule) in ingress.rules.iter().enumerate() {
        let router_name_base = rule.domain_name.replace(".", "-");
        
        for (j, svc) in rule.services.iter().enumerate() {
             let router_name = format!("{}-{}-{}", router_name_base, i, j);
             writeln!(file, "    {}:", router_name)?;
             
             let path_rule = if svc.prefix == "/" {
                 String::new()
             } else {
                 format!(" && PathPrefix(`{}`)", svc.prefix)
             };
             
             writeln!(file, "      rule: \"Host(`{}`){}\"", rule.domain_name, path_rule)?;
             writeln!(file, "      service: service-{}-{}", router_name_base, j)?;
             
             if has_tls {
                 writeln!(file, "      entryPoints:")?;
                 writeln!(file, "        - websecure")?;
                 writeln!(file, "      tls:")?;
                 if use_le {
                     writeln!(file, "        certResolver: myresolver")?;
                 }
             } else {
                 writeln!(file, "      entryPoints:")?;
                 writeln!(file, "        - web")?;
             }
             
             if svc.strip_prefix && svc.prefix != "/" {
                  writeln!(file, "      middlewares:")?;
                  writeln!(file, "        - strip-{}-{}", router_name_base, j)?;
             }
        }
    }
    
    writeln!(file, "  services:")?;
    for rule in ingress.rules.iter() {
        let router_name_base = rule.domain_name.replace(".", "-");
        for (j, svc) in rule.services.iter().enumerate() {
             writeln!(file, "    service-{}-{}:", router_name_base, j)?;
             writeln!(file, "      loadBalancer:")?;
             writeln!(file, "        servers:")?;
             writeln!(file, "          - url: \"http://{}:{}/\"", svc.service_name, svc.port)?;
        }
    }
    
    Ok(())
}
