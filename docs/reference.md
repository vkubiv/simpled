# Reference

Complete field reference for `appspec.yaml` and `envspec.yaml`.

---

## appspec.yaml

Describes an application — its services, environment variables, secrets, and configuration files. The file lives in the root of your application repository and is bundled into the app artifact.

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Application name. Must match the `application.name` in `envspec.yaml`. |
| `version` | string | yes | Semantic version (e.g. `1.3.52`). Applied as the image tag for all `app_services`. |
| `environment` | object | no | Environment variable definitions. See [Environment](#environment). |
| `app_services` | map | no | Services versioned with the app. See [Services](#services). |
| `extra_services` | map | no | Third-party services with their own image versions. See [Services](#services). |
| `configs` | map | no | Named groups of configuration files. See [Configs](#configs). |
| `secrets` | map | no | Secret definitions. See [Secrets](#secrets). |
| `volumes` | list | no | Named volumes available to services. See [Volumes](#volumes). |

---

### Environment

```yaml
environment:
  external:
    - VAR_NAME
    - VAR_WITH_DEFAULT=value
  optional:
    - OPTIONAL_VAR
    - OPTIONAL_WITH_DEFAULT=value
  relative:
    - URL_VAR=/some/path
  internal:
    - INTERNAL_VAR=value
```

| Section | Description |
|---------|-------------|
| `external` | Required variables. Deployment fails if not provided and no default set. |
| `optional` | Optional variables. Deployment succeeds even if missing. |
| `relative` | URL variables. Value is prepended with the deployment's primary host domain at deploy time. Can be overridden by the environment. |
| `internal` | Fixed variables set by the app author. Identical across all environments. |

All sections accept entries in two forms:
- `VAR_NAME` — no default; must be supplied by the environment (for `external`) or left unset (for `optional`)
- `VAR_NAME=default` — has a default value

---

### Services

`app_services` and `extra_services` share the same structure. The difference: `app_services` images are automatically tagged with the app version; `extra_services` must specify the version in the `image` field.

```yaml
app_services:
  service-name:
    type: public | internal | job
    image: org/image-name          # no tag — version appended automatically
    variants:
      arm:
        image: org/image-name-arm  # alternative image for this variant
    export:
      host: myapp
      prefix: /
    environment:
      - VAR_NAME
      - $all
      - VAR_NAME=override-value
    configs:
      - config-name: /mount/path
    secrets:
      - secret_name:
      - secret_name:
        variable: ENV_VAR_NAME
      - secret_name:
        path: /custom/path/name
    ports:
      - 8080
    volumes:
      - named-volume:/container/path
      - ./host/path:/container/path

extra_services:
  postgres:
    type: internal
    image: postgres:16             # version required for extra_services
  redis:
    type: internal
    image: redis:7-alpine
```

#### Service fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | `public`, `internal`, or `job`. |
| `image` | string | yes | Docker image name. For `app_services`, omit the tag. For `extra_services`, include the tag. |
| `variants` | map | no | Alternative images. Selected with `variant` in `envspec.yaml`. |
| `export` | object | no | Default `host` and `prefix` for this service. |
| `environment` | list | no | Variables to inject. Use `$all` to pass everything. Individual entries can override with `NAME=value`. |
| `configs` | list | no | Config groups to mount. Format: `- config-name: /mount/path`. |
| `secrets` | list | no | Secrets to provide. See below. |
| `ports` | list | no | Ports to expose (Docker). Informational in Kubernetes. |
| `volumes` | list | no | Volume mounts. Named volumes must be declared in the top-level `volumes:` list. |

#### Service types

| Type | Description                                                                                                                                          | Runs |
|------|------------------------------------------------------------------------------------------------------------------------------------------------------|------|
| `public` | Exposed externally via ingress. Must have `host` and `prefix` configured in `envspec.yaml`. Use for any service that responds to HTTP requests.      | Continuously |
| `internal` | No ingress routing. Use for background workers, queue consumers, and support services (databases, caches) that do not serve HTTP requests extarnaly. | Continuously |
| `job` | Runs once per deployment. Not accessible from other services. Use for database migrations and one-time setup tasks.                                  | Once |

#### Secret mount options

```yaml
secrets:
  - secret_name:                    # mount at /secrets/secret_name (default)
  - secret_name:
    path: /custom/path/secret_name  # mount at custom path
  - secret_name:
    variable: ENV_VAR_NAME          # inject as environment variable
```

---

### Configs

Named groups of files that can be mounted into services.

```yaml
configs:
  data:
    - country_payments.json
    - exercises.json
  certs:
    - ca.pem
```

Mount in a service:
```yaml
configs:
  - data: /app/data      # mounts all files in the group at /app/data/
  - certs: /app/certs
```

The deployment's `envspec.yaml` maps each config name to a directory on disk containing those files.

---

### Secrets

Declare all secrets the application may use:

```yaml
secrets:
  db_password:
  redis_password:
  api_key:
```

Values are never stored in `appspec.yaml`. They are provided by the deployment environment in `envspec.yaml`.

---

### Volumes

Named volumes must be declared before services can use them:

```yaml
volumes:
  - postgres-data
  - uploads
```

Host paths (`./relative` or `/absolute`) do not need to be declared.

---

## envspec.yaml

Describes an environment — where and how to deploy applications. Lives in your deployment repository or environment-specific directory.

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | no | `k8s` (default), `docker`, or `local`. |
| `swarm_mode` | bool | no | Enable Docker Swarm mode. Only valid when `type: docker`. |
| `registry` | map | no | Image registry prefix mappings. |
| `ingress` | object | yes | Ingress (load balancer) configuration. |
| `deployments` | map | yes | Named deployment configurations. |

---

### type

```yaml
type: k8s      # Kubernetes — generates manifests/ directory
type: docker   # Docker standalone or Swarm — generates docker-deploy/ directory
type: local    # Local development — generates local_env/docker-compose.yaml
```

---

### registry

Maps image name prefixes to registry hostnames:

```yaml
registry:
  mycompany: registry.mycompany.com
  allimb: allimbacr.azurecr.io
```

An image `mycompany/api` becomes `registry.mycompany.com/mycompany/api` at deploy time.

---

### ingress

```yaml
ingress:
  name: my-ingress
  type: nginx | traefik    # docker only; defaults to traefik
  hosts:
    hostname-alias: domain.com
    multi-domain-alias:
      - www.domain.com
      - domain.com
  tls:
    disable: true          # no TLS
    secret: tls-secret     # existing TLS secret (k8s)
    letsencrypt:
      email: ops@co.com
      server: https://...  # optional; defaults to Let's Encrypt production
```

`hosts` maps abstract names (used in `services[].host`) to real domain names. For local environments, use `localhost:port`.

#### TLS options (mutually exclusive)

| Option | Description |
|--------|-------------|
| `disable: true` | No TLS. HTTP only. |
| `secret: name` | Use an existing Kubernetes TLS secret. |
| `letsencrypt` | Provision via Let's Encrypt (cert-manager). Kubernetes only. |

---

### deployments

Each deployment configures one application in this environment.

```yaml
deployments:
  deployment_name:
    primary_host: hostname-alias
    application:
      name: app-name
      version: ^1.0.0
      extra:
        - extra-services.yaml
    environment: path/to/vars.env
    undockerized_environment: path/to/native.env
    configs:
      config-name: ./path/to/files
    secrets:
      secret_name:
        value: literal        # local dev only
        env: ENV_VAR_NAME     # read from shell environment
        file: ./path/to/file  # read from file
    defaults:
      replicas: 2
      resources:
        requests:
          memory: "128Mi"
          cpu: "250m"
        limits:
          memory: "512Mi"
          cpu: "1000m"
    services:
      service-name:
        host: hostname-alias
        prefix: /path
        prefixes:
          "/path1":
            strip: true | false
        strip_prefix: true | false
        variant: variant-name
        replicas: 3
        resources:
          requests:
            memory: "256Mi"
            cpu: "500m"
          limits:
            memory: "1Gi"
            cpu: "2000m"
```

#### Deployment fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `primary_host` | string | yes | Ingress host alias used as the base URL for `relative` environment variables. |
| `application` | object | yes | App name, version constraint, and optional extra service files. |
| `environment` | string | no | Path to a `.env` file with variable values. |
| `undockerized_environment` | string | no | Path to a `.env` file for services running outside Docker. |
| `configs` | map | no | Maps config names to directories containing the config files. |
| `secrets` | map | no | Provides values for the secrets declared in `appspec.yaml`. |
| `defaults` | object | no | Default replica count and resource limits applied to all services. |
| `services` | map | no | Per-service overrides (routing, replicas, resources, variants). |

#### application

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Must match `name` in `appspec.yaml`. |
| `version` | string | no | SemVer range (e.g. `^1.0.0`, `>=1.2.0 <2.0.0`). Deployment fails if app version doesn't satisfy. |
| `extra` | list | no | Additional YAML files with `extra_services` to include for this deployment only. |

#### secrets

Each secret must match a name declared in `appspec.yaml`. Exactly one source must be provided:

| Source | Description |
|--------|-------------|
| `value: literal` | Inline value. Use only for local development. |
| `env: VAR_NAME` | Read from the named shell environment variable at deploy time. |
| `file: ./path` | Read from a file at deploy time. |

#### service overrides

| Field | Type | Description |
|-------|------|-------------|
| `host` | string | Ingress host alias. Required for `public` services. |
| `prefix` | string | URL path prefix. Required for `public` services (unless set via `export`). |
| `prefixes` | map | Multiple prefix rules, each with optional `strip: bool`. Mutually exclusive with `prefix`. |
| `strip_prefix` | bool | Whether to strip the prefix before forwarding to upstream. Default `true`. |
| `variant` | string | Image variant to use (must be declared in `appspec.yaml`). |
| `replicas` | int | Number of pod/container replicas. Overrides `defaults.replicas`. |
| `resources` | object | CPU/memory requests and limits. Overrides `defaults.resources`. |

---

## CLI reference

### `simpled app-bundle verify`

Run from the application directory. Validates `appspec.yaml` and checks that Docker images exist for all services.

```
simpled app-bundle verify
```

### `simpled app-bundle version`

Prints the application version from `appspec.yaml`.

```
simpled app-bundle version
```

### `simpled app-bundle create`

Creates a deployable app bundle.

```
simpled app-bundle create [OPTIONS]

Options:
  --registry <PREFIX=HOST>     Map image prefix to registry (repeatable)
  --push-images                Tag and push images to registry
  --upload-bundle-to <TARGET>  Upload bundle: github-release
  --github-repo <OWNER/REPO>   GitHub repository
  --github-tag-prefix <PREFIX> Prefix for GitHub release tag
```

### `simpled prepare-deployment`

Generates deployment manifests from `envspec.yaml` and an app bundle.

```
simpled prepare-deployment <DEPLOYMENT_NAME> [OPTIONS]

Options:
  --app-bundle, --bundle <PATH>        Path to app bundle (.tar.gz or directory)
  --app-version, --version <VERSION>   Expected app version (for verification)
  --download-bundle-from <SOURCE>      Download bundle: github-release
  --github-repo <OWNER/REPO>           GitHub repository
  --github-tag-prefix <PREFIX>         Prefix for GitHub release tag
```

Must be run from the directory containing `envspec.yaml`.

Required environment variables for secrets with `env:` source must be set before running this command.

### `simpled local run`

Generates Docker Compose and starts local services with a reverse proxy.

```
simpled local run [OPTIONS]

Options:
  --exclude <SERVICE>  Exclude a service (repeatable)
  --path <PATH>        Path to the project directory (default: current dir)
```

### `simpled secrets set`

Manages secrets for a named environment.

```
simpled secrets set <ENV_NAME> [OPTIONS]

Options:
  --file <PATH>  Load secrets from file
```

---

## Generated output

### Kubernetes (`type: k8s`)

Output directory: `k8s/`

| File | Description |
|------|-------------|
| `deployment-<service>.yaml` | Kubernetes Deployment |
| `service-<service>.yaml` | Kubernetes Service |
| `ingress.yaml` | Ingress resource with all routing rules |
| `configmap-<name>.yaml` | ConfigMap for each config group |
| `secret-<name>.yaml` | Secret for each secret |
| `cluster-issuer.yaml` | Let's Encrypt ClusterIssuer (if configured) |

### Docker standalone (`type: docker`, no swarm)

Output directory: `docker-deploy/<deployment-name>/`

| File/Dir | Description |
|----------|-------------|
| `deploy.sh` | Script to pull images and start containers |
| `envs/<service>.env` | Per-service environment variable files |
| `configs/` | Configuration files |
| `secrets/` | Secret files |

### Docker Swarm (`type: docker`, `swarm_mode: true`)

Output directory: `docker-deploy/`

| File | Description |
|------|-------------|
| `<deployment>.yaml` | Docker Compose stack file for `docker stack deploy` |
| `ingress/` | Traefik or nginx ingress stack |

### Local (`type: local`)

Output directory: `local_env/`

| File | Description |
|------|-------------|
| `docker-compose.yaml` | Compose file for all services |
| `<service>/.env` | Per-service environment variable file |
| `undockerized.env` | Variables for services run outside Docker |
