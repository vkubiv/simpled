//! `GET /.well-known/simpled/version` on every gateway host: which application,
//! version and deployment were last deployed behind it.
//!
//! The gateway is shared by every deployment of an env spec and rewritten by
//! each of their deploys, so it cannot hold the version itself — deploying one
//! deployment would overwrite the others'. Each deployment instead gets a tiny
//! service of its own that answers with its document, and the gateway only
//! routes the path to the right one. Locally there is one deployment, and the
//! local gateway answers directly.

use crate::resolved_spec::{ServiceResolvedSpec, VersionRoute};
use crate::spec::*;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PATH: &str = "/.well-known/simpled/version";
pub const IMAGE: &str = "nginx:1.31-alpine";
const PORT: u16 = 80;
const CONF_VARIABLE: &str = "SIMPLED_VERSION_CONF";

pub fn service_name(deployment: &str) -> String {
    format!("simpled-version-{}", deployment)
}

/// The JSON a deployment's version endpoint answers with. `deployed_at` is when
/// the deployment was prepared, so a redeploy of one version is still visible.
pub fn document(application: &str, version: &str, deployment: &str) -> String {
    let deployed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    serde_json::json!({
        "application": application,
        "version": version,
        "deployment": deployment,
        "deployed_at": deployed_at,
    })
    .to_string()
}

/// The nginx container that serves `document` on every path. Its config comes
/// in through an environment variable and is written out by `printenv`, so no
/// file has to reach the deploy target and nothing in it needs `$`-escaping.
/// On Kubernetes `ports` becomes the Service's port; on Docker it would publish a
/// host port, so there the port is only exposed to the gateway.
pub fn service(deployment: &str, document: &str, env_type: &DeploymentEnvType) -> ServiceResolvedSpec {
    let on_k8s = matches!(env_type, DeploymentEnvType::K8S);
    // nginx single-quoted strings take backslash escapes; JSON has no line breaks.
    let body = document.replace('\\', "\\\\").replace('\'', "\\'");
    let conf = format!(
        "server {{ listen {PORT} default_server; location / {{ default_type application/json; \
         add_header Cache-Control no-store always; return 200 '{body}'; }} }}"
    );
    ServiceResolvedSpec {
        service_type: ServiceType::Internal,
        is_app_service: false,
        full_name: service_name(deployment),
        image: IMAGE.to_string(),
        environment_variables: vec![EnvVariable {
            name: CONF_VARIABLE.to_string(),
            value: conf,
        }],
        undockerized_environment_variables: vec![],
        configs: vec![],
        secrets: vec![],
        ports: if on_k8s {
            vec![ServicePort {
                external: PORT,
                internal: PORT,
            }]
        } else {
            vec![]
        },
        expose: if on_k8s { vec![] } else { vec![PORT.to_string()] },
        volumes: vec![],
        command: Some(ServiceCommand::Exec(vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            format!("printenv {CONF_VARIABLE} > /etc/nginx/conf.d/default.conf && exec nginx -g 'daemon off;'"),
        ])),
        entrypoint: None,
        healthcheck: None,
        depends_on: vec![],
        resources: ResourcesSpec {
            replicas: 1,
            requests: ResourceLimits {
                memory: "16Mi".to_string(),
                cpu: "10m".to_string(),
            },
            limits: ResourceLimits {
                memory: "32Mi".to_string(),
                cpu: "50m".to_string(),
            },
        },
        working_dir: None,
        excluded: false,
    }
}

/// Which deployment answers on each served domain: the only one routing to it,
/// or, when several share a domain, the one whose primary host it belongs to.
/// A shared domain that is no deployment's primary host gets no endpoint.
pub fn routes(env_spec: &DeploymentEnvironmentSpec) -> Vec<VersionRoute> {
    let domains_of = |alias: &str| -> Vec<&String> {
        env_spec
            .ingress
            .hosts
            .iter()
            .filter(|h| h.name == alias)
            .flat_map(|h| h.domain_names.iter())
            .collect()
    };

    let mut owners: BTreeMap<&String, Vec<&DeploymentSpec>> = BTreeMap::new();
    for dep in &env_spec.deployments {
        for service in dep.services.values() {
            for route in &service.routes {
                let alias = route.host.as_deref().unwrap_or(&dep.primary_host);
                for domain in domains_of(alias) {
                    let deps = owners.entry(domain).or_default();
                    if !deps.iter().any(|d| d.name == dep.name) {
                        deps.push(dep);
                    }
                }
            }
        }
    }

    owners
        .into_iter()
        .filter_map(|(domain, deps)| {
            let owner = match deps.as_slice() {
                [only] => Some(*only),
                many => {
                    let primary: Vec<_> = many
                        .iter()
                        .filter(|d| domains_of(&d.primary_host).contains(&domain))
                        .collect();
                    match primary.as_slice() {
                        [only] => Some(**only),
                        _ => None,
                    }
                }
            }?;
            Some(VersionRoute {
                domain_name: domain.clone(),
                deployment_name: owner.name.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_spec;

    fn owners(blog_primary: &str) -> Vec<(String, String)> {
        let raw = format!(
            r#"
type: docker
gateway:
  tls: {{ disable: true }}
  hosts:
    web: shop.example.com
    blog: blog.example.com
    shared: api.example.com
deployments:
  shop:
    primary_host: web
    application: {{ name: shop }}
    services:
      api:
        hosts:
          web: {{ prefix: / }}
          shared: {{ prefix: /shop }}
  blog:
    primary_host: {blog_primary}
    application: {{ name: blog }}
    services:
      site:
        hosts:
          blog: {{ prefix: / }}
          shared: {{ prefix: /blog }}
"#
        );
        let dir = tempfile::tempdir().unwrap();
        routes(&env_spec(&raw, dir.path()))
            .into_iter()
            .map(|r| (r.domain_name, r.deployment_name))
            .collect()
    }

    fn pair(domain: &str, deployment: &str) -> (String, String) {
        (domain.to_string(), deployment.to_string())
    }

    #[test]
    fn each_domain_answers_for_the_deployment_serving_it() {
        assert_eq!(
            owners("blog"),
            vec![pair("blog.example.com", "blog"), pair("shop.example.com", "shop")]
        );
    }

    #[test]
    fn a_shared_domain_answers_for_the_deployment_whose_primary_host_it_is() {
        assert_eq!(
            owners("shared"),
            vec![
                pair("api.example.com", "blog"),
                pair("blog.example.com", "blog"),
                pair("shop.example.com", "shop")
            ]
        );
    }

    #[test]
    fn the_service_serves_the_document_with_quotes_escaped_for_nginx() {
        let doc = document("it's", "1.0.0", "prod");
        let service = service("prod", &doc, &DeploymentEnvType::K8S);
        assert_eq!(service.ports.len(), 1);
        let docker = super::service(
            "prod",
            &doc,
            &DeploymentEnvType::Docker(DockerSpecificSpec {
                swarm_mode: true,
                ingress_type: DockerIngressType::Nginx,
            }),
        );
        assert!(docker.ports.is_empty(), "a host port would clash with the gateway");
        assert_eq!(docker.expose, vec!["80".to_string()]);
        assert_eq!(service.full_name, "simpled-version-prod");
        let conf = &service.environment_variables[0].value;
        assert!(conf.contains(r#"return 200 '{"application":"it\'s","#), "{conf}");
        assert!(!conf.contains('\n'), "an env file cannot carry a line break");
    }
}
