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
  relative:
    - URL_VAR=/some/path
  internal:
    - INTERNAL_VAR=value
```

| Section | Description |
|---------|-------------|
| `external` | Required variables. Deployment fails if not provided and no default set. |
| `optional` | Optional variables. No defaults allowed. Deployment succeeds even if missing; a service that references one just does not get the variable. |
| `relative` | URL variables. Value is prepended with the deployment's primary host domain at deploy time. Can be overridden by the environment. |
| `internal` | Fixed variables set by the app author. Identical across all environments. |

All sections accept entries in two forms:
- `VAR_NAME` — no default; must be supplied by the environment (for `external`) or left unset (for `optional`)
- `VAR_NAME=default` — has a default value (not allowed in `optional`)

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
    depends_on:
      - other-service

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
| `depends_on` | list | no | Services that must be running before this one starts. Drives the ordering of the generated Docker deploy scripts and the local compose file; ignored for Kubernetes. Cycles are rejected. |

#### Service types

| Type | Description                                                                                                                                          | Runs |
|------|------------------------------------------------------------------------------------------------------------------------------------------------------|------|
| `public` | Exposed externally via ingress. Must have `host` and `prefix` configured in `envspec.yaml`. Use for any service that responds to HTTP requests.      | Continuously |
| `internal` | No ingress routing. Use for background workers, queue consumers, and support services (databases, caches) that do not serve HTTP requests extarnaly. | Continuously |
| `job` | Runs once per deployment. Not accessible from other services. Use for database migrations and one-time setup tasks.                                  | Once |

#### Jobs and deployment ordering

A job runs to completion, so it is not deployed like the other services. Every
Docker deployment that contains at least one job runs in three phases:

1. **Start what the jobs need.** The transitive `depends_on` closure of all jobs is
   started first, and the deploy waits for it to be ready.
2. **Run the jobs**, in dependency order when one job depends on another. A job
   that fails aborts the deploy — nothing else is rolled out.
3. **Deploy the rest of the stack**, once the migrations have been applied.

A job that declares no `depends_on` is treated as depending on *every*
long-running service, so phase 1 starts the whole stack. Declare the dependencies
explicitly (`depends_on: [primary-db]`) to keep phase 1 small and to guarantee that
nothing serves traffic against a schema the migration has not touched yet.

Readiness in phase 1 means:

| Environment | Waits for |
|-------------|-----------|
| Swarm | `docker stack deploy --detach=false` convergence — tasks running, and healthy when the service declares a `healthcheck` |
| Standalone Docker | container started; additionally polls `docker inspect` until healthy for dependencies that declare a `healthcheck` (override the 300s limit with `HEALTH_TIMEOUT`) |
| Local | `docker compose` `depends_on` conditions: `service_healthy` when the dependency declares a `healthcheck`, otherwise `service_started` |

Give a database or other job dependency a `healthcheck`; without one, "ready" only
means the container was started, which is rarely enough for a migration.

In Swarm the job is not part of the stack file at all. `docker stack deploy` cannot
express a run-to-completion service (compose's `deploy.mode` has no
`replicated-job`), so a job placed in a stack would be deployed as a normal
replicated service that never converges, and a crashed migration would be
indistinguishable from a successful one. The generated `deploy.sh` instead creates
the job with `docker service create --mode replicated-job --restart-condition none
--detach=false`, waits for the task to reach a terminal state (override the 600s
limit with `JOB_TIMEOUT`), prints its logs, removes the service, and exits non-zero
unless the task reports `Complete`. Overriding `entrypoint` on a job requires a
Docker CLI that supports `docker service create --entrypoint` (25.0+); `command`,
env files, secrets, configs and volume mounts work on any version.

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

## envspec.yaml / localenv.yaml

Describes an environment — where and how to deploy applications. Lives in your deployment repository or environment-specific directory.

`simpled` looks for the env spec file in this order: `envspec.yaml`, `envspec.yml`, `localenv.yaml`, `localenv.yml`. The first file found is used.

Use `envspec.yaml` for Kubernetes and Docker environments. Use `localenv.yaml` for local development — in that file the `type` field defaults to `local` and can be omitted.

### Top-level fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | no | `k8s`, `docker`, or `local`. Required in `envspec.yaml`; defaults to `local` in `localenv.yaml`. |
| `swarm_mode` | bool | no | Enable Docker Swarm mode. Only valid when `type: docker`. |
| `registry` | map | no | Image registry prefix mappings. Not valid for `local`. |
| `gateway` | object | yes | Gateway (load balancer) configuration. The deprecated alias `ingress` is still accepted with a warning. |
| `deployments` | map | yes | Named deployment configurations. |

---

### type

```yaml
type: k8s      # Kubernetes — generates manifests/ directory
type: docker   # Docker standalone or Swarm — generates docker-deploy/ directory
type: local    # Local development — generates local_env/docker-compose.yaml
```

When using `localenv.yaml`, `type` can be omitted:

```yaml
# localenv.yaml — type: local is the default
gateway:
  hosts:
    myapp: localhost:8080
deployments:
  ...
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

### secrets_folder

Local-only. Set `secrets_folder` on a deployment and simpled will load each empty secret value from a file in that folder named after the secret.

```yaml
# localenv.yaml
deployments:
  myapp_local:
    secrets_folder: ./secrets
    secrets:
      db_password:           # reads ./secrets/db_password
      redis_password:        # reads ./secrets/redis_password
      api_key: real-key      # used as-is, folder not consulted
```

This keeps sensitive values out of the spec file while still having a simple, declarative local config. The `secrets/` directory can be git-ignored.

`secrets_folder` is not valid for `k8s` or `docker` environments.

---

### AWS Secrets Manager

A deployment secret can name a secret in AWS Secrets Manager instead of carrying its value:

```yaml
# envspec.yaml
deployments:
  myapp_prod:
    secrets:
      db_password:
        aws: prod/myapp/db      # secret name or full ARN
        jq: .password           # optional; the secret holds JSON, take one field
      sendgrid_apikey:
        aws: prod/myapp/sendgrid
```

Unlike `env` and `file`, an `aws` secret is **not** read while `simpled prepare-deployment` runs. That command usually runs in CI, and the directory it produces is copied to the deploy target, so a value resolved there would travel through the build artifact and whatever transport carries it. Instead simpled generates `fetch-secrets.sh` next to `deploy.sh`, and the lookup happens on the machine that runs the deploy:

```
docker-deploy/
├── deploy.sh           # sources fetch-secrets.sh before it starts anything
├── fetch-secrets.sh
└── ...
```

Nothing extra to run — `deploy.sh` sources it. For `k8s`, `manifests/fetch-secrets.sh` is generated instead and must be run against the target cluster **before** `kubectl apply -f manifests/`; it creates the Secrets with `kubectl create secret generic`.

The files it writes end up as `0644` owned by the user that ran the deploy — the same as the secrets `prepare-deployment` writes itself. Both halves of that matter on the target: a Docker deploy is run with sudo, so without the `chown` the files would belong to root and the unprivileged user that copies the next deployment in could no longer overwrite them; and a container that bind-mounts a secret runs as a user of its own, so a mode narrower than `0644` means a service that does not run as root cannot read its own secret. Each file is created readable by its owner only and widened once the value is in it.

Requirements on the deploy target:

- The **AWS CLI** on `PATH`. Credentials, region and profile come from its own environment — an instance role, `AWS_REGION`, `AWS_PROFILE`, and so on. The generated script does not configure any of them.
- **jq** on `PATH`, but only when at least one secret sets `jq`.

Every secret is read, and checked, before the script writes the first file or creates the first Kubernetes Secret — one that does not resolve stops the deploy instead of leaving the others half-applied. A lookup fails the deploy when:

- the secret does not exist in the account and region the target is configured for, or the target may not read it;
- it exists but carries no string value — an empty secret, or one that holds only binary data;
- a `jq` filter selects a key the secret's JSON does not have, or one whose value is `null`. Without that check jq prints the four characters `null` and the service would start with `null` as its credential;
- a `jq` filter cannot be applied at all, because the filter is invalid or the secret's value is not JSON.

jq's own error output is discarded rather than printed: a parse error quotes the input it choked on, which is the secret. The messages the script prints name the secret and the id it tried, never the value.

Notes:

- `jq` is only valid together with `aws`, and `aws` cannot be combined with `env` or `file`.
- `$secret(name)` cannot reference an `aws` secret: env files are written when the deployment is prepared, and the value does not exist yet. Mount the secret on the service with `variable:` instead.
- For `local` deployments there is no deploy target to defer to, so the lookup runs on your machine while the environment is resolved.

---

### gateway

```yaml
gateway:
  name: my-gateway          # optional; defaults to "gateway"
  type: nginx | traefik     # docker only; defaults to traefik
  hosts:
    hostname-alias: domain.com
    multi-domain-alias:
      - www.domain.com
      - domain.com
  body_limit: 10m         # optional; max request body for every route
  redirects:
    - from: domain.com      # a domain, or a list of them
      to: www.domain.com    # bare domain or full URL
      permanent: true       # optional; 301 when true (default), 302 when false
  tls:
    disable: true           # no TLS
    secret: tls-secret      # existing TLS secret (k8s)
    letsencrypt:
      email: ops@co.com
      server: https://...   # optional; defaults to Let's Encrypt production
```

`hosts` maps abstract names (used in `services[].host`) to real domain names. For local environments, use `localhost:port`.

#### redirects

Domains the gateway answers on only to send the visitor somewhere else, typically the bare apex pointing at the `www` host that serves the app:

```yaml
gateway:
  hosts:
    website: www.somesite.com
  redirects:
    - from: somesite.com
      to: www.somesite.com
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `from` | string or list | yes | Source domain(s). Written in full, not as a `hosts` alias. |
| `to` | string | yes | Destination. A bare domain takes the gateway's own scheme (`https://` when TLS is on); a value containing `://` is used as-is. |
| `permanent` | bool | no | `true` (default) sends 301, `false` sends 302. |

The path and query string are preserved: `somesite.com/pricing?ref=x` → `www.somesite.com/pricing?ref=x`.

Redirect sources are included in the gateway's certificate, since the redirect itself has to be served over HTTPS. On Kubernetes each destination becomes its own `Ingress` carrying the ingress-nginx `permanent-redirect`/`temporal-redirect` annotation; on Docker it becomes an nginx `server` block or a Traefik `redirectRegex` middleware; locally the gateway matches the `Host` header, so a redirect can share a port with routed services.

A domain may not appear under both `hosts` and `redirects`, and a source may not be listed twice — both make routing ambiguous and are rejected.

#### body_limit

Maximum size of a request body the gateway will accept. Set it on `gateway` for every route, and on an individual service under `deployments[].services[]` to override that default:

```yaml
gateway:
  body_limit: 10m
  hosts:
    myapp: app.myapp.com

deployments:
  prod:
    services:
      upload-svc:
        host: myapp
        prefix: /upload
        body_limit: 500m
```

Values use nginx notation: a plain byte count, or a number with a `k`/`m`/`g` suffix (a trailing `b` is allowed, so `2mb` reads the same as `2m`). `0` means no limit. Omitting it everywhere leaves the gateway's own default in place — 1 MB for nginx and ingress-nginx, unlimited for Traefik.

How it is applied per target:

| Target | Mechanism |
|--------|-----------|
| `k8s` | `nginx.ingress.kubernetes.io/proxy-body-size`. The annotation covers a whole Ingress, so routes with different limits are split into separate Ingress objects (`<gateway>--limit-<bytes>`); the primary object keeps the gateway name and owns the TLS certificate. |
| `docker` + nginx | `client_max_body_size` inside each `location` block. |
| `docker` + traefik | A `buffering` middleware with `maxRequestBodyBytes`. Traefik has no non-buffering size cap, so a limited route spools the request body before forwarding it. |
| `local` | The gateway rejects a request whose `Content-Length` exceeds the limit with `413`. A chunked request that declares no length is passed through. |

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
    secrets_folder: ./secrets   # local only; omit on k8s/docker
    secrets:
      literal_secret: the-value   # local dev only
      from_env:
        env: ENV_VAR_NAME         # read from shell environment
      from_file:
        file: ./path/to/file      # read from file
      from_aws:
        aws: prod/myapp/db        # read from AWS Secrets Manager on the deploy target
        jq: .password             # optional; for secrets holding a JSON document
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
| `extends` | string | no | Name of another deployment to inherit from. See [extends](#extends). |
| `abstract` | bool | no | If `true`, this deployment is a template used only as an `extends` base and is not deployed itself. See [extends](#extends). |
| `primary_host` | string | yes¹ | Gateway host alias used as the base URL for `relative` environment variables. |
| `application` | object | yes¹ | App name, version constraint, and optional extra service files. |
| `environment` | list \| string | no | Either a list of `NAME=value` entries, or a path to a `.env` file with variable values. |
| `undockerized_environment` | list \| string | no | Same forms as `environment`, for services running outside Docker. See [undockerized_environment](#undockerized_environment). |
| `configs` | map | no | Maps config names to directories containing the config files. |
| `secrets` | map | no | Provides values for the secrets declared in `appspec.yaml`, from a literal, `env`, `file` or `aws`. See [AWS Secrets Manager](#aws-secrets-manager). |
| `secrets_folder` | string | no | Path to a folder of secret files. Only valid for `local`. See [secrets_folder](#secrets_folder). |
| `defaults` | object | no | Default replica count and resource limits applied to all services. |
| `services` | map | no | Per-service overrides (routing, replicas, resources, variants). |

¹ Required unless supplied by an `extends` base, or omitted on an `abstract` template.

#### extends

A deployment may inherit from another deployment in the same env spec with `extends`. This keeps shared configuration in one place when several deployments (e.g. `staging` and `prod`) differ only in a few fields.

Merge rules, applied to the base (the deployment named by `extends`) and the child (the deployment declaring it):

- **Scalar fields** (`primary_host`, `application`, `defaults`, `secrets_folder`) are taken from the base unless the child sets them, in which case the child's value replaces the base's entirely.
- **Map fields** (`configs`, `secrets`, `services`) are unioned. Keys that appear only in the base or only in the child are kept as-is; a key present in both takes the child's value. For `services`, a shared key is merged field-by-field, so a child can override just `replicas` on a service while inheriting its `host`, `prefix`, and `resources` from the base.
- **Environment lists** (`environment`, `undockerized_environment`) are merged per variable. The base's variables are all inherited in order; a variable the child redefines takes the child's value in place, and variables only the child declares are appended. So a child listing two variables overrides exactly those two and keeps the rest. A `.env` file path cannot be merged with a list — if either side uses one, the child's value replaces the base's entirely.

`extends` chains are followed to any depth, and cycles are rejected with an error.

Mark a deployment `abstract: true` to use it purely as a template. Abstract deployments are never deployed and may omit fields that are otherwise required (such as `primary_host`), so a base can hold only the fields worth sharing.

```yaml
deployments:
  common:
    abstract: true                # template only, not deployed
    application:
      name: myapp
      version: ^1.0.0
    environment:
      - REGION=us-east-2
      - LOG_LEVEL=debug
    defaults:
      replicas: 2
    services:
      api:
        host: web
        prefix: /
  staging:
    extends: common
    primary_host: staging
  prod:
    extends: common
    primary_host: web
    environment:
      - LOG_LEVEL=warn            # overrides LOG_LEVEL, keeps common's other variables
    services:
      api:
        replicas: 5               # inherits host/prefix from common, overrides replicas
```

#### application

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Must match `name` in `appspec.yaml`. |
| `version` | string | no | SemVer range (e.g. `^1.0.0`, `>=1.2.0 <2.0.0`). Deployment fails if app version doesn't satisfy. |
| `extra` | list | no | Additional YAML files with `extra_services` to include for this deployment only. |

#### secrets

Each secret must match a name declared in `appspec.yaml`. Exactly one source must be provided:

| Form | Description |
|------|-------------|
| `secret_name: "literal"` | Inline string value. Local development only. |
| `secret_name:` or `secret_name: ''` | No value — load from `secrets_folder` file. Requires `secrets_folder` to be set. |
| `secret_name:` + `env: VAR_NAME` | Read from the named shell environment variable at deploy time. |
| `secret_name:` + `file: ./path` | Read from a file at deploy time. |

```yaml
secrets:
  db_password: localpass         # inline value (local only)
  api_key:                       # load from secrets_folder/api_key
  redis_password:
    env: REDIS_PASSWORD          # read from env var
  admin_cert:
    file: ./secrets/admin.pem    # read from file
```

#### undockerized_environment

Path to a `.env` file whose variables are exposed to services you run outside Docker (e.g. a process started from your IDE). The values layer on top of `environment` and are written to `<service>/undockerized.env` for `local` environments only. A service that sets [`working_dir`](#working_dir) instead gets these variables as a `.env` file in its working directory.

For `local` environments, a `.env.local` file located next to `localenv.yaml` overrides these variables: any name present in both wins from `.env.local`, and names not in `undockerized_environment` are appended. This file is intended for per-developer, machine-specific overrides — keep it out of version control (e.g. add it to `.gitignore`). It has no effect on `k8s` or `docker` environments.

```bash
# .env.local — gitignored
DB_CONNECTION_STRING=Host=localhost;Port=5432;Database=myapp
```

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
| `working_dir` | string | Local only. Directory of a host-run (non-dockerized) service. See [working_dir](#working_dir). |

#### working_dir

Local only. Marks a service as host-run (started by hand or from your IDE, outside Docker) and points at the directory it runs from. When set, `simpled` writes that service's setup into `working_dir` instead of `local_env/<service>/`:

- the [undockerized environment](#undockerized_environment) is written as a `.env` file in `working_dir` (rather than `local_env/<service>/undockerized.env`),
- the service's secrets are copied alongside it: `variable:` secrets are merged into the `.env` file, and `path:`/file secrets are written as files relative to `working_dir`.

This pairs with `simpled local only-extra`, which runs the gateway and extra services in Docker while you run the app service yourself. Point your process at `working_dir/.env` and it has the same environment and secrets the dockerized service would get.

```yaml
# localenv.yaml
deployments:
  myapp_local:
    services:
      api:
        host: web
        prefix: /
        ports:
          - "8080:80"
        working_dir: ../api   # run the api here; .env + secrets written into it
```

`working_dir` is not valid for `k8s` or `docker` environments.

Two services in the same deployment cannot share a `working_dir`: each one writes its own `.env` and secrets there, so they would overwrite each other. `simpled` rejects the spec with an error naming both services. Paths are compared after normalization, so `./api`, `api` and `api/` all count as the same directory. Different deployments may reuse a directory, since only one runs at a time.

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
  --no-create-repos            Do not create missing Amazon ECR repositories
  --upload-bundle-to <TARGET>  Upload bundle: github-release
  --github-repo <OWNER/REPO>   GitHub repository
  --github-tag-prefix <PREFIX> Prefix for GitHub release tag
  --version-suffix <SUFFIX>    Label appended to the app version (side branches)
```

#### Amazon ECR

ECR, unlike Docker Hub or GHCR, does not create a repository on first push and instead fails with:

```
name unknown: The repository with name 'myapp/web' does not exist in the registry with id '123456789012'
```

When the registry host is an ECR one (`<account>.dkr.ecr.<region>.amazonaws.com`), `--push-images`
therefore creates each missing repository before pushing to it, using the account and region from the
host. This needs the AWS CLI on `PATH` and the `ecr:DescribeRepositories` and `ecr:CreateRepository`
permissions; existing repositories are left untouched, so only the describe permission is used once
every repository exists. Pass `--no-create-repos` to turn this off and manage repositories yourself.

#### Version suffix

`--version-suffix` labels a bundle built from a side branch, so it does not collide
with the mainline build of the same `appspec.yaml` version:

```bash
simpled app-bundle create --version-suffix big-refactor
```

For `version: 1.0.2` this produces version `1.0.2+big-refactor` and:

| | Value |
|---|---|
| Image tags | `registry/myapp/api:1.0.2-big-refactor` |
| Bundle file | `myapp.1.0.2-big-refactor.tar.gz` |
| Release tag | `1.0.2-big-refactor` (after `--github-tag-prefix`) |
| `version:` in the bundled appspec | `1.0.2+big-refactor` |

The suffix is stored as semver build metadata, which version requirements ignore — a
deployment pinned to `version: "^1.0"` still accepts the bundle. Because `+` is not a
legal character in a docker tag, everything that is a tag or a file name uses `-`
instead.

`appspec.yaml` in your working tree is **not** modified; only the copy written into the
bundle carries the suffixed version. That is what makes `prepare-deployment` resolve the
suffixed image tags without needing the flag repeated — see below.

A suffix may be any string; characters outside `[A-Za-z0-9]` are replaced with `-`, so a
branch name can be passed through directly:

```bash
simpled app-bundle create --version-suffix "${{ github.ref_name }}"   # feature/big-refactor
# -> 1.0.2+feature-big-refactor
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

There is no `--version-suffix` here: a suffixed bundle carries its version internally, so
pass the suffixed version to `--app-version` and the images resolve accordingly.

```bash
simpled prepare-deployment staging \
  --download-bundle-from github-release \
  --github-repo myorg/myapp \
  --app-version 1.0.2-big-refactor
```

Either spelling of the version is accepted — `1.0.2-big-refactor` or `1.0.2+big-refactor`.

Required environment variables for secrets with `env:` source must be set before running this command.

### `simpled local run`

Generates Docker Compose and starts local services with a reverse proxy.

```
simpled local run [OPTIONS]

Options:
  --exclude <SERVICE>      Exclude a service (repeatable)
  --path <PATH>            Path to the project directory (default: current dir)
  --deployment <NAME>      Deployment to run. Required when the env spec defines
                           more than one deployment.
```

An env spec may define multiple deployments, but only one can run locally at a
time. When a single deployment is defined it is used automatically; when more
than one is defined you must pick one with `--deployment <name>`.

The deployments not picked take no part in the run: they are dropped before
validation, so local deployments are free to share domains, paths, and ports
with each other. This is what makes [`extends`](#extends) useful locally — a
variant deployment can inherit its sibling's services wholesale and change only
the few fields it cares about.

### `simpled local only-extra`

Runs the gateway and only extra services, skipping all app services. Useful when you want to run app services outside Docker (e.g. for debugging) while still having the gateway and supporting infrastructure available.

```
simpled local only-extra [OPTIONS]

Options:
  --path <PATH>        Path to the project directory (default: current dir)
  --deployment <NAME>  Deployment to run. Required when the env spec defines
                       more than one deployment.
```

### `simpled local generate-config`

Writes `local_env/docker-compose.yaml` and per-service `.env` files without starting the gateway or running Docker Compose.

```
simpled local generate-config [OPTIONS]

Options:
  --path <PATH>        Path to the project directory (default: current dir)
  --deployment <NAME>  Deployment to generate config for. Required when the env
                       spec defines more than one deployment.
```

### `simpled secrets set`

Manages secrets for a named environment.

```
simpled secrets set <ENV_NAME> [OPTIONS]

Options:
  --file <PATH>  Load secrets from file
```

### `simpled docs`

Prints the documentation embedded in the binary — these guides ship inside `simpled`
itself, so they are available in any project without checking this repository out.

```
simpled docs                        List the available topics
simpled docs <TOPIC>                Print a topic
simpled docs <TOPIC> --outline      List the topic's section headings and anchors
simpled docs <TOPIC> --section <S>  Print one section
simpled docs search <QUERY>         Search every topic
```

Topics are `agent`, `tutorial`, `reference`, `examples` and `cicd`. An unambiguous
prefix works, so `simpled docs ref` prints the reference.

`--section` matches either the anchor printed by `--outline` (`secret-mount-options`) or
any substring of a heading (`secret`), in which case every matching section is printed.
`search` reports each hit as `topic#anchor`, which can be passed straight back to
`--section`.

### `simpled init-agent`

Writes `.claude/skills/simpled/SKILL.md` into a project, so a coding agent working there
discovers `simpled docs` and looks fields up instead of guessing at them.

```
simpled init-agent [OPTIONS]

Options:
  --path <PATH>  Project directory to write into (default: current dir)
  --force        Overwrite an existing skill file
  --stdout       Print the skill instead of writing it
```

The generated file is a plain Markdown document; adapt it freely, or pipe `--stdout`
into whatever convention your agent uses (`AGENTS.md`, `CLAUDE.md`, …).

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
| `deploy.sh` | Deploy script: ingress, then the job phases, then the stack. The full stack file is deployed with `--prune`, so services removed from the spec are removed from the swarm on the next deploy |
| `<deployment>/docker-compose.yaml` | Stack file with all long-running services (jobs are not part of it) |
| `<deployment>/docker-compose.deps.yaml` | Stack file with only the services the jobs depend on (phase 1); written only when that is a strict subset |
| `<deployment>/<service>/` | Per-service `.env`, config and secret files |
| `ingress/` | Traefik or nginx ingress stack |

### Local (`type: local`)

Output directory: `local_env/`

The compose file sets its project name to `<application.name>_local`, so several applications can be run locally on the same machine without one taking over the other's project.

| File | Description |
|------|-------------|
| `docker-compose.yaml` | Compose file for all services |
| `<service>/.env` | Per-service environment variable file |
| `<service>/undockerized.env` | Variables for services run outside Docker (unless the service sets `working_dir`) |
| `<working_dir>/.env` | Environment and secrets for a host-run service that sets `working_dir` |
