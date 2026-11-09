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
| `ports` | list | no | Ports to expose. On Kubernetes they become the Service's ports; a service without ports gets no Service object. |
| `volumes` | list | no | Volume mounts. Named volumes must be declared in the top-level `volumes:` list. |
| `depends_on` | list | no | Services that must be running before this one starts. Drives the ordering of the generated Docker deploy scripts and the local compose file; ignored for Kubernetes. Cycles are rejected. For a `job`, leaving it out means every long-running service, and an explicit `[]` means nothing in the stack (its dependencies are external). A deployment may replace the list in `envspec.yaml`. |

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

A job that does not mention `depends_on` is treated as depending on *every*
long-running service, so phase 1 starts the whole stack. Declare the dependencies
explicitly (`depends_on: [primary-db]`) to keep phase 1 small and to guarantee that
nothing serves traffic against a schema the migration has not touched yet. On a
fresh database the fallback is also a deadlock: a service that cannot start without
its tables keeps phase 1 from converging, so the migration never runs.

An explicit empty list, `depends_on: []`, is the opposite statement: the job needs
nothing from the stack, because what it talks to (a managed database, say) lives
outside it. Phase 1 then starts nothing and the job runs first.

Which of the two is right is usually an environment question — the same database is
a stack service on the dev box and RDS in production — so a deployment can replace a
service's list in `envspec.yaml`:

```yaml
# appspec.yaml: by itself the job needs nothing from the stack
app_services:
  migrate:
    type: job
    depends_on: []

# envspec.yaml, dev: here the database is an extra service, start it first
deployments:
  dev:
    services:
      migrate:
        depends_on: [primary-db]
```

The deployment's list replaces the app's (it is not merged), is validated against
that deployment's services, and is inherited through `extends` like any other
per-service field.

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

On Kubernetes a job becomes a `batch/v1` Job with `restartPolicy: Never` and
`backoffLimit: 0`, so a failed migration shows up as a failed Job rather than being
retried. A Job's pod template is immutable and the image changes with every release,
so each deploy gets a Job of its own, named `<service>-<deploy date>` and
garbage-collected a day after it finishes. `kubectl apply` submits the whole directory
at once, so nothing waits for the job there: the phases above are a Docker feature, and
`depends_on` is not enforced on Kubernetes.

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

On Docker Swarm, the env spec says where each named volume lives, and relative
paths are rejected unless `multi_node: false`; see
[Volumes on Docker Swarm](#volumes-on-docker-swarm).

A bind source that does not exist makes Swarm reject the task, so the generated
`deploy.sh` creates every host path a deployment mounts on the node it runs on, before
the stack starts: relative paths under the deployment directory, absolute paths only
when nothing is there yet. An existing path of any kind — a directory, a file, a
socket such as `/var/run/docker.sock` — is left as it is.

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
| `multi_node` | bool | no | Swarm only, default `true`: validate as if the swarm had several nodes. `false` keeps the environment on one node. See [multi_node](#multi_node). |
| `target_platform` | string | no | Swarm only: `linux-amd64` (default) or `linux-arm64`, the platform of the deploy target, which the shipped simpled binary must run on. See [Deploy steps on Docker Swarm](#deploy-steps-on-docker-swarm). |
| `registry` | map | no | Image registry prefix mappings. In `localenv.yaml`, used only when running a [released bundle](#running-a-released-bundle-locally). |
| `gateway` | object | yes | Gateway (load balancer) configuration. The deprecated alias `ingress` is still accepted with a warning. |
| `deployments` | map | yes | Named deployment configurations. |
| `networks` | map | no | Docker only: networks deployments share. A deployment reaches no other deployment unless both join one. See [networks](#networks). |

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

In a `localenv.yaml` the map is ignored until a run names a released bundle: app
services normally run `mycompany/api:latest` from the local Docker cache.

#### Running a released bundle locally

`simpled test`, `simpled local run` and `simpled local generate-config` take the
same bundle options as `prepare-deployment`. With one, the appspec comes from the
bundle instead of the project directory, and every app service runs
`<registry>/<image>:<bundle version>` — the image `app-bundle create --push-images`
published — instead of `<image>:latest`. Extra services are unchanged.

```yaml
# localenv.yaml
registry:
  mycompany: 123456789012.dkr.ecr.us-east-2.amazonaws.com/
```

```bash
simpled test e2e   --download-bundle-from github-release   --github-repo mycompany/myapp   --github-tag-prefix myapp-v   --app-version 1.4.0
```

This is how a repository that does not hold the app's source — a mobile client,
another service — runs its tests against an exact release: it keeps its own
`localenv.yaml` and `testspec.yaml` and names the version. The download goes to
`.simpled-bundles/` beside the env spec and is reused on the next run; keep that
directory out of version control. `GITHUB_TOKEN` must be able to read the
release, and Docker must be logged in to the registry (for ECR:
`aws ecr get-login-password | docker login --username AWS --password-stdin <host>`),
since compose pulls the images on first use. An app image without a `registry`
entry for its namespace is an error. The deployment's `application.version`
requirement, when set, is checked against the bundle as in a deploy.

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

`echo "$KEY" > secrets/api_key` is the normal way to write one of these files, and an editor adds the same trailing newline, so it is not part of the value: see [Whitespace and empty values](#whitespace-and-empty-values).

`secrets_folder` is not valid for `k8s` or `docker` environments.

---

### secrets_env_prefix

Resolves every valueless secret from the environment, so a deployment with twenty secrets does not need twenty `env:` lines:

```yaml
# localenv.yaml
deployments:
  myapp_local:
    secrets_env_prefix: E2E_SECRET_
    secrets:
      openai_api_key:        # reads $E2E_SECRET_OPENAI_API_KEY
      db_password:           # reads $E2E_SECRET_DB_PASSWORD
      api_key: real-key      # used as-is, the environment is not consulted
```

The variable is the prefix plus the secret's own name, and **the match ignores case**: secrets are named in lower case and environment variables in upper, and both spellings — and anything between — find each other. Two variables that differ only in case are an error rather than a coin toss.

Unlike `secrets_folder`, this is valid in every environment type. It is the same lookup a per-secret `env:` source does, with the name derived instead of written down, and it happens where the deployment is prepared.

**With `secrets_folder`**, the folder is tried first: a file that exists wins, and the prefix covers every secret that has no file. That is what makes one value overridable locally — drop the file in, leave the variables alone:

```yaml
    secrets_folder: ./secrets
    secrets_env_prefix: E2E_SECRET_
    secrets:
      openai_api_key:        # ./secrets/openai_api_key, else $E2E_SECRET_OPENAI_API_KEY
```

---

### secrets_json

One JSON document holding every secret, a field per secret:

```json
{ "openai_api_key": "sk-...", "db": { "password": "hunter2" } }
```

```yaml
# localenv.yaml
deployments:
  myapp_local:
    secrets_json: ./secrets.json
    secrets:
      openai_api_key:        # .openai_api_key
      db_password:
        jq: .db.password     # a field that is not named after the secret
```

The filter defaults to the field named after the secret, quoted when the name is not an identifier (`.["api-key"]` would be a subtraction to jq unquoted). A field the document does not have, or one whose value is `null`, is not an error here — it falls through to the next source in the chain below.

**Only a field path** — `.a`, `.a.b`, `."a b"` — is understood. simpled resolves it itself, so a local stack needs no jq installed; a filter that does more than select a field belongs with an `aws` source, where jq runs on the deploy target.

---

### secrets_aws

The same document, in AWS Secrets Manager, for every secret the deployment does not otherwise provide:

```yaml
# envspec.yaml
deployments:
  myapp_prod:
    secrets_aws: prod/myapp/bundle
    secrets:
      openai_api_key:        # prod/myapp/bundle, .openai_api_key
      db_password:
        jq: .db.password
      legacy_key:
        aws: prod/myapp/legacy   # its own secret, read whole
```

It is an [`aws` source](#aws-secrets-manager) with the id filled in and `jq` defaulting to the secret's own name, so everything that section says applies — above all that the lookup happens on the deploy target, not where the deployment is prepared. One lookup is made per secret, so a bundle of twelve secrets is twelve `get-secret-value` calls at deploy time.

A secret that writes its own `aws:` is read **whole** unless it also writes `jq:`. Only the deployment-wide documents default to a field, so a secret that holds a single value keeps working unchanged.

Together, `secrets_json` for local and `secrets_aws` for the real environments let both specs list the secret names and nothing else.

---

### The fallback chain

A secret with no value and no source of its own is looked for in each of these that the deployment configures, in this order:

| | Source | Read |
|--|--------|------|
| 1 | [`secrets_folder`](#secrets_folder) | `<folder>/<name>`, the whole file |
| 2 | [`secrets_json`](#secrets_json) | the field of the document |
| 3 | [`secrets_env_prefix`](#secrets_env_prefix) | `<prefix><name>` in the environment, ignoring case |
| 4 | [`secrets_aws`](#secrets_aws) | the field of the document, on the deploy target |

A secret that sets `jq:` skips 1 and 3, which hold one raw value each and cannot select a field.

The order puts the most specific first: a file dropped into the folder overrides the document for that one value, without touching anything else. `secrets_aws` is last because it is the only one that cannot be tried where the deployment is prepared.

A value that is in none of them fails with every place that was consulted named at once:

```
Secret 'stripe_key' has no value. Tried: secrets/stripe_key (no such file),
secrets.json (no .stripe_key), $CI_SECRET_stripe_key (not set)
```

A source that *has* the secret but holds nothing — an empty file, an empty field — is an error rather than a fall-through.

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

#### aws-secrets.local.json

Where simpled reads an `aws` secret **on this machine** (a `local` env spec, and the controller's token in [`simpled status`](#simpled-status) and `logs`), an `aws-secrets.local.json` next to the env spec stands in for Secrets Manager, for developers without AWS access. It is a JSON object from secret id, exactly as the spec writes it, to the secret's value:

```json
{
  "dev/simpled-controller": { "controller_read_token": "..." },
  "dev/myapp/db": { "password": "local-only" }
}
```

- The `jq` path applies to the value as it would to the SecretString, and the result is what `jq -er` gives on the deploy target: a string as it is, a number or `true` as jq prints it. `false`, `null` and a missing key are errors, as `jq -e` fails on them; so is an object or array, which jq prints indented. Only a path to one field (`.a`, `.a.b`, `."a b"`) is understood.
- A value may also be the SecretString as Secrets Manager returns it: a string, or a JSON document in a string. Without `jq` it must be a string, and not the text `None`, which the AWS CLI prints for a secret with no string value.
- A secret the file does not list is read with the AWS CLI, as without the file. A listed secret the file cannot answer is an error, not a fall-back. A file that is broken as a whole (not valid JSON, not an object) does not block the AWS CLI: simpled warns once that it ignored it, and when the AWS CLI fails too, the error names both. A filter that is not a path to one field cannot be stood in for; such a secret needs the AWS CLI.
- The file is read once per command, however many secrets it answers.
- Only the env spec's own directory is looked in. The `.local.` name is the usual never-commit convention: add `*.local.json` (or the file) to `.gitignore` and keep it `chmod 600`; on Linux and macOS simpled warns when others may read it. No error quotes its content.

`prepare-deployment` for a remote target does **not** read it: its `aws` secrets stay with `fetch-secrets.sh` on the deploy target. When the file is there and the deployment has `aws` secrets, `prepare-deployment` says so on stderr, and goes on. To prepare with those values anyway, for example to check a deployment with real secrets, pass the file explicitly: `--aws-secrets-file aws-secrets.local.json` (any path; the same shape). Every `aws` secret is then read from it while preparing and **written into the output** like an `env` secret, so no `fetch-secrets.sh` is needed for them; a secret it does not list is an error. Keep that output on your machine.

Notes:

- A secret has one source: `env`, `file`, `aws`, `json_file` or `json_env`. `jq` goes with `aws`, [`json_file` or `json_env`](#json_file-and-json_env), [`secrets_json`](#secrets_json) or [`secrets_aws`](#secrets_aws) — never with `env` or `file`, which carry one raw value.
- An `aws` source written on the secret itself is read whole unless it sets `jq`. The deployment-wide [`secrets_aws`](#secrets_aws) is the one that defaults to a field named after the secret.
- `$secret(name)` cannot reference an `aws` secret: env files are written when the deployment is prepared, and the value does not exist yet. Mount the secret on the service with `variable:` instead.
- For `local` deployments there is no deploy target to defer to, so the lookup runs on your machine while the environment is resolved.

---

### gateway

```yaml
gateway:
  name: my-gateway          # optional; defaults to "gateway"
  type: traefik             # docker only, optional; Traefik is the only Docker gateway
  hosts:
    hostname-alias: domain.com
    multi-domain-alias:
      - www.domain.com
      - domain.com
  body_limit: 10m         # optional; max request body for every route
  trusted_proxies:          # docker only, optional; proxies whose X-Forwarded-For is believed
    - 10.0.0.0/8
  access_log: true          # docker only, optional; one log line per request (JSON)
  global: true              # swarm only, optional; one gateway task on every node
  placement:                # swarm only, optional; where the gateway task(s) run
    constraints:
      - node.role == worker
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
    upstream: true          # docker + traefik; TLS ends at a load balancer in front
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

Redirect sources are included in the gateway's certificate, since the redirect itself has to be served over HTTPS. On Kubernetes each destination becomes its own `Ingress` carrying the ingress-nginx `permanent-redirect`/`temporal-redirect` annotation; on Docker it becomes a Traefik `redirectRegex` middleware; locally the gateway matches the `Host` header, so a redirect can share a port with routed services.

A domain may not appear under both `hosts` and `redirects`, and a source may not be listed twice — both make routing ambiguous and are rejected.

#### version endpoint

On Kubernetes and locally, every gateway host answers `GET /.well-known/simpled/version` with what was last deployed behind it:

```json
{"application":"shop","version":"1.2.3","deployment":"prod","deployed_at":1790945826}
```

On Kubernetes `deployed_at` is when `prepare-deployment` ran (Unix seconds), so a redeploy of the same version is visible too. A host belongs to a deployment:

- a host only one deployment routes to belongs to that deployment;
- a host several deployments route to belongs to the one whose `primary_host` it is, and gets no endpoint when that is none of them or more than one.

| Target | Who answers |
|---|---|
| Kubernetes | every deployment ships `simpled-version-<deployment>` (nginx, a few MB of memory), routed by a separate Ingress, `<gateway>--simpled-version`, so the main Ingress's `rewrite-target` does not apply to it. No `health`. |
| Local | the local gateway itself, with no extra container |
| Docker | nothing on the app's hosts: the [controller](#the-simpled-controller-docker) answers versions and health under `/simpled/` on `controller.host`. |

#### The simpled controller (Docker)

The gateway is shared by every deployment of an env spec and rewritten by each of their deploys, so it cannot hold the versions itself. On Docker, one controller per environment answers for all of them. It reads the state of the running services through the Docker API, and finds a deployment's services by the labels `prepare-deployment` puts on each of them: `simpled.deployment`, `simpled.application`, `simpled.version`, `simpled.deployed_at` and `simpled.service`. On Swarm these are service labels, so a deploy that only moves `deployed_at` restarts nothing.

Every Docker env spec names the host the controller is served on, with the top-level `controller` block. It sits beside `gateway`, not under it: the controller belongs to the environment. `host` is an alias from `gateway.hosts`, and everything the controller serves is under `/simpled/` on each of that host's domains. The gateway strips the prefix before forwarding.

```yaml
gateway:
  hosts:
    web: shop.example.com
    ops: ops.example.com

controller:
  host: ops                     # required on Docker
  read_token:                   # optional: the private API
    aws: prod/simpled/tokens    # any source a deployment secret takes
    jq: .read
  manage_token:
    env: SIMPLED_MANAGE_TOKEN
```

| Field | Description |
|-------|-------------|
| `host` | Required on Docker. The host alias the controller is served on, under `/simpled/`. It may be an app's host too: a service routed at `/simpled` or below on the same domain fails validation, anything else is left alone. |
| `read_token` | Reads the private API. Every source a deployment secret takes (`env`, `file`, `aws`, `json_env`, …); an `aws` token is fetched on the deploy target by `fetch-secrets.sh`. At least 32 characters, for example `openssl rand -hex 32`. |
| `manage_token` | Everything the read token can, and the write routes when they come. |

| | Swarm | Standalone |
|---|---|---|
| Runs as | service `simpled_controller` of the `simpled` stack, on a manager node | container `simpled-controller`, started by `deploy.sh` before the gateway |
| Image | `ghcr.io/vkubiv/simpled:<version>`, the version that prepared the deployment; `SIMPLED_CONTROLLER_IMAGE=<image> ./deploy.sh` overrides it | the same |
| Config | `gateway/controller/controller.json`, as a Swarm config | `controller/controller.json`, bind-mounted |
| Network | `simpled_control`, an overlay network only the gateway and the controller are on | `simpled_control`, a bridge network, the same way |
| Docker access | `/var/run/docker.sock`. As root, since the controller may run on another manager than the one running `deploy.sh`, whose `docker` group may have another number. Read-only filesystem, no capabilities. | `/var/run/docker.sock`, as user 65532 with the socket's group (`stat -c %g`). Read-only filesystem, no capabilities. |

App containers are not on `simpled_control` and reach the controller only through the gateway's routes. The Docker socket is effectively root on the host. On Swarm with `tls.letsencrypt` the controller also writes through it: it creates the certificate secrets and updates the gateway service.

##### Health and versions (public)

| Route | Returns |
|---|---|
| `GET /simpled/health` | The whole environment: the gateway, every certificate, and every deployment that has services. |
| `GET /simpled/health/{deployment}` | One deployment. |
| `GET /simpled/version/{deployment}` | `{"application","version","deployment","deployed_at"}`; `deployed_at` is when `deploy.sh` ran (Unix seconds), so a redeploy of the same version is visible too. |

Both health routes answer 200 whenever the controller could read Docker, whatever `status` says, even `down`: the HTTP status is about the controller, and the health is in the body. They answer 503 only when the controller cannot read Docker (or, for `/simpled/health`, the gateway service), with whatever document it could build. An uptime check alarms on `status` in the body, or on a 503 or no answer for the controller itself. A deployment the env spec does not name and Docker does not run is a 404.

```json
{"status":"healthy","converged":true,"version":"1.2.3"}
```

| `status` | When |
|---|---|
| `down` | a service that should run has no task (Swarm) or container (standalone) running, or the deployment has no services |
| `degraded` | a service runs fewer tasks than it should, or a container fails its healthcheck |
| `healthy` | neither |

`converged` is true once the rollout has finished: no Swarm update or rollback is in progress, and every service is at the deployed version. A deploy pipeline can poll the deployment's health until it reports `converged`, `healthy` and the version it just shipped, which [`simpled wait`](#simpled-wait) does. Polling `version` only proves the stack was updated. Jobs and services scaled to zero do not count.

On Swarm with `tls.letsencrypt`, a deployment's health also has `tls`, the worst certificate of the domains its services are routed on: `valid`, `expiring` (within 7 days of expiry, so renewal, which starts at 30 days, has kept failing), `self-signed` (the placeholder: the certificate has not been issued yet, for example because the domain's DNS does not point at the gateway) or `expired` (issued, and past its expiry: browsers fail). Anything but `valid` makes a `healthy` deployment `degraded`; `services_status` then has the status of the services alone. Right after the first deploy, that lasts until the controller has issued the certificates, usually under a minute.

The environment's health is the worst of its parts:

```json
{
  "status": "degraded",
  "gateway": {"status": "healthy", "desired": 3, "running": 3},
  "tls": "self-signed",
  "deployments": {
    "prod": {"status": "healthy", "converged": true, "version": "1.2.3"},
    "admin": {"status": "degraded", "converged": true, "version": "0.4.0"}
  }
}
```

`gateway` is `down` with nothing running and `degraded` with fewer tasks than wanted, as a global gateway missing a node. `tls`, with Let's Encrypt on Swarm, is the worst certificate of every domain, and anything but `valid` degrades the environment.

##### Private API

The private API reports every deployment's services. Its routes exist only when `controller` has a token.

**The read token is a credential**, not a viewer password: what the API returns, and the logs it will serve, can be as sensitive as the secrets that leak into them. The tokens reach the controller as gateway Swarm secrets on Swarm, so every manager has them, and as files under `controller/` on standalone Docker; `controller.json` only says where they are.

Every request needs `Authorization: Bearer <token>`. A missing or wrong token gets 401 and a token without enough rights 403. A client that fails 10 times gets 429 instead of 401 until a minute has passed without a failure. Past the tenth failure, one request from it at a time is answered, after a second per failure beyond the tenth, up to ten seconds, whatever token it presents; the others are refused with 429 at once, their token unread, so the guesses it can make are bounded however many connections it opens. The client is the address the gateway forwards, which behind a proxy is everyone's: a valid token from there is refused only while another request from there is waiting, and passes once that one is through.

| Route | Returns |
|---|---|
| `GET /simpled/api/v1/status` | Every deployment: its version document, its health (as `/simpled/health/{deployment}`), and its services with desired and running counts. |
| `GET /simpled/api/v1/deployments/{deployment}/services` | Per service: name, image, version, desired, running, unhealthy and whether an update is in progress. A deployment the controller does not know is a 404. |
| `GET /simpled/api/v1/deployments/{deployment}/services/{service}/tasks` | Swarm: the service's last 20 tasks, newest first, each with its node, state, desired state, since when, exit code and error, which say why a service does not converge. Standalone: its containers with state, status and health. |
| `GET /simpled/api/v1/deployments/{deployment}/services/{service}/logs` | The service's log as `text/plain`, streamed; on Swarm aggregated across nodes. `tail=200` (default; at most 10000, which `all` stands for; on Swarm Docker applies it to each task), `since=10m` (`30s`, `2h`, `1d`, or Unix seconds), `follow=1` to keep streaming, `timestamps=1`. |
| `GET /simpled/api/v1/gateway` | The gateway's tasks per node, or its container; with Let's Encrypt on Swarm, each domain's certificate: `issuer` (`acme`, `self-signed` or `none`) and `expires_at` (Unix seconds). |
| `GET /simpled/api/v1/nodes` | Swarm: every node's hostname, role, availability, state, address, labels and whether it leads. A 404 on standalone Docker. |

`{deployment}` and `{service}` are the names from the specs (`prod`, `api`), not Swarm's. A name the controller does not own is a 404 either way, so names cannot be probed.

```bash
curl -H "Authorization: Bearer $SIMPLED_TOKEN" https://ops.example.com/simpled/api/v1/status
```

##### CloudWatch

With `controller.cloudwatch` the controller publishes the environment's health to Amazon CloudWatch every minute and keeps the alarms on it, emailing them through SNS. No Lambda or prober is needed: when the server, Docker or the controller dies, the metrics stop, and the `silent` alarm fires on the missing data.

```yaml
controller:
  host: ops
  cloudwatch:
    alarm_email: [ops@example.com]   # optional: who the alarms email
    # name: shop-prod                 # optional: gateway.name by default
    # region: us-east-2               # optional: the instance's region by default
```

| Field | Description |
|-------|-------------|
| `name` | The `Env` dimension, and part of every alarm's name (`simpled:<name>:...`). 1 to 64 letters, digits, `-`, `_` or `.`. Defaults to `gateway.name`, so give environments in one AWS account different gateway names, or set it. |
| `region` | Where the metrics, alarms and topic live. Defaults to `AWS_REGION`, then the region of the instance the controller runs on. |
| `alarm_email` | Addresses subscribed to the SNS topic `simpled-<name>-health`, which every alarm notifies when it fires and when it recovers. AWS emails each new address once to confirm; nothing reaches it until that link is clicked. An address removed here is unsubscribed (once confirmed). Without `alarm_email` the alarms exist with no actions. |

Metrics, namespace `Simpled`, dimension `Env` (plus `Deployment`), every 60 seconds; health is 0 healthy, 1 degraded, 2 down:

| Metric | Dimensions | Value |
|---|---|---|
| `Health` | `Env` | The worst of the gateway, every certificate and every deployment of the env spec, one without services counting as down. |
| `Health` | `Env`, `Deployment` | That deployment's health, as `/simpled/health/<deployment>`. |
| `GatewayHealth` | `Env` | The gateway alone. |
| `CertDaysLeft` | `Env` | Days until the earliest certificate expires; 0 for one not issued yet. Only with certificates the controller issues (Swarm with `tls.letsencrypt`). |

Nothing is published while the controller cannot read Docker. Alarms, kept by the controller at start and every 6 hours:

| Alarm | Fires when |
|---|---|
| `simpled:<name>:silent` | No `Health` for 3 minutes (missing data is breaching): the server, Docker or the controller is down, or it cannot reach CloudWatch. |
| `simpled:<name>:deployment:<deployment>` | That deployment is down for 3 minutes. |
| `simpled:<name>:gateway` | The gateway is down for 2 minutes. |
| `simpled:<name>:unhealthy` | The environment is not healthy for 15 minutes: longer than a rollout, so a stuck one, missing replicas, a certificate, or anything down. |
| `simpled:<name>:certificate` | A certificate has had under 14 days left, or none issued, for an hour (renewal starts at 30). |

The controller deletes alarms under `simpled:<name>:` that the env spec no longer calls for, such as a removed deployment's, and nothing else. In money: a metric per deployment plus three, at $0.30 a month each, and $0.10 an alarm.

**Credentials.** The controller signs its calls itself (no AWS SDK) with `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (and `AWS_SESSION_TOKEN`) if set, else the EC2 instance role through IMDSv2. In a container IMDSv2 is one network hop further away, so the instance needs a hop limit of 2, or the controller logs that it cannot reach the metadata service and `silent` fires:

```sh
aws ec2 modify-instance-metadata-options --instance-id <id>   --http-tokens required --http-put-response-hop-limit 2 --http-endpoint enabled
```

On Swarm the controller runs on a manager node; every manager's role needs this policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": ["cloudwatch:PutMetricData", "cloudwatch:DescribeAlarms"], "Resource": "*" },
    { "Effect": "Allow", "Action": ["cloudwatch:PutMetricAlarm", "cloudwatch:DeleteAlarms"],
      "Resource": "arn:aws:cloudwatch:*:*:alarm:simpled:*" },
    { "Effect": "Allow",
      "Action": ["sns:CreateTopic", "sns:Subscribe", "sns:Unsubscribe", "sns:ListSubscriptionsByTopic"],
      "Resource": "arn:aws:sns:*:*:simpled-*-health" }
  ]
}
```

`sns:Unsubscribe` is authorized on the topic. The controller logs what it changed (`docker service logs simpled_controller` or `docker logs simpled-controller`), and a failure once until it recovers.

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

Values use nginx's `client_max_body_size` notation: a plain byte count, or a number with a `k`/`m`/`g` suffix (a trailing `b` is allowed, so `2mb` reads the same as `2m`). `0` means no limit. Omitting it everywhere leaves the gateway's own default in place — 1 MB for ingress-nginx, unlimited for Traefik.

How it is applied per target:

| Target | Mechanism |
|--------|-----------|
| `k8s` | `nginx.ingress.kubernetes.io/proxy-body-size`. The annotation covers a whole Ingress, so routes with different limits are split into separate Ingress objects (`<gateway>--limit-<bytes>`); the primary object keeps the gateway name and owns the TLS certificate. |
| `docker` | A `buffering` middleware with `maxRequestBodyBytes`. Traefik has no non-buffering size cap, so a limited route spools the request body before forwarding it. |
| `local` | The gateway rejects a request whose `Content-Length` exceeds the limit with `413`. A chunked request that declares no length is passed through. |

#### trusted_proxies

IPs or CIDRs (IPv4 or IPv6) of the proxies in front of the gateway, such as a CDN or a load balancer. A request arriving from one of them keeps its `X-Forwarded-For`, so the apps behind the gateway see the original client rather than the proxy:

```yaml
gateway:
  trusted_proxies:
    - 130.176.0.0/16
    - 2600:9000::/28
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `trusted_proxies` | list of strings | no | Each entry an IP address or a CIDR. Anything else fails validation. Unset or empty trusts no one, and the gateway config is what it was without the field. |

| Target | Mechanism |
|--------|-----------|
| `docker` | `forwardedHeaders.trustedIPs` on the `web` entrypoint and, with TLS, on `websecure`. Without it Traefik replaces an incoming `X-Forwarded-For` with the address it was connected from. |
| `k8s` | Rejected. The ingress controller owns this (ingress-nginx: `proxy-real-ip-cidr` and `use-forwarded-headers` in its ConfigMap). |
| `local` | Rejected. |

Under Docker Swarm the gateway publishes its ports in host mode, so it sees the real peer address to begin with; see [Docker Swarm](#docker-swarm-type-docker-swarm_mode-true).

#### access_log

Traefik's access log: one line per request, for every host and app, on the gateway's stdout next to Traefik's own log. It is read with `docker service logs simpled_gateway` (Swarm), `docker logs simpled-gateway` (standalone) or `simpled logs`, and a log driver or agent ships it on, e.g. to CloudWatch Logs.

```yaml
gateway:
  access_log: true              # JSON
  # access_log: {format: common}  # Common Log Format
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `access_log` | `true`, `false` or `{format: json\|common}` | no | `true` and `{}` are `json`. Unset or `false` is off, and the gateway config is what it was without the field. |

A `json` line carries, among others, `ClientHost` (the client IP), `RequestHost`, `RequestMethod`, `RequestPath`, `RouterName` and `ServiceName` (which app), `DownstreamStatus`, `Duration` (nanoseconds) and `request_X-Forwarded-For`. Every other request header is dropped from the log. `common` is one Common Log Format line, client IP first and the duration last in milliseconds, with the router name but no host.

The client IP is the address the gateway was connected from, or the original client from `X-Forwarded-For` when that address is in [`trusted_proxies`](#trusted_proxies). From any other address Traefik strips the header, so it cannot fake the IP. Through several proxies `ClientHost` is the whole `X-Forwarded-For` list, the original client first.

| Target | Mechanism |
|--------|-----------|
| `docker` | `accessLog` in `traefik.yml`, with no `filePath` (stdout), JSON with `fields.headers` keeping only `X-Forwarded-For`. |
| `k8s` | Rejected. The ingress controller owns its access log. |
| `local` | Rejected. |

#### TLS options (mutually exclusive)

| Option | Description |
|--------|-------------|
| `disable: true` | No TLS. HTTP only. |
| `secret: name` | Use an existing Kubernetes TLS secret. Kubernetes only; rejected on `docker`. |
| `letsencrypt` | Provision via Let's Encrypt: cert-manager on Kubernetes, the [simpled controller](#certificates-on-docker-swarm) on Docker Swarm, Traefik's own ACME client on standalone Docker. `server` sets another ACME directory, such as Let's Encrypt's staging one. |
| `upstream: true` | TLS is terminated in front of the gateway, by a load balancer or CDN holding the certificate. `docker` only. See [tls.upstream](#tlsupstream). |

#### tls.upstream

The gateway serves plain HTTP and the load balancer in front of it holds the certificate. Clients still use `https://`, so `relative` environment variables and redirect targets keep that scheme; only the gateway itself changes:

- Traefik has the `web` entrypoint on `:80` only: no `websecure`, no `certificatesResolvers`/ACME, and no HTTP→HTTPS redirect (behind an HTTP listener it would loop). Routers carry no `tls`.
- The gateway publishes only port 80 (host mode on Swarm, `-p 80:80` standalone).
- Traefik's ping is enabled on `web`: `GET /ping` on port 80 answers `200 OK`, for use as the load balancer's health check. The path is fixed.

| Combination | Result |
|-------------|--------|
| `upstream` with `letsencrypt` or `secret` | Validation error: the gateway holds no certificate. |
| `upstream` with `disable: true` | Validation error. |
| `upstream` on `k8s` or `local` | Validation error. |
| `upstream` with empty `trusted_proxies` | Allowed, with a warning: Traefik then discards the `X-Forwarded-For` and `X-Forwarded-Proto` the balancer sends, so the apps see its address and `http`. |

About `/ping`: Traefik's built-in ping router is ``PathPrefix(`/ping`)``, which would also take `/pingback` and `/ping/x` on every domain, so simpled turns on `manualRouting` and adds its own router, `simpled-ping`, with the rule ``Path(`/ping`)``, no `Host` matcher and the highest priority, on the `web` entrypoint. It answers whatever `Host` header arrives, which matters because an ALB health check sends the target's IP as `Host`, and it takes only the exact path `/ping` from the app routes, on any domain; `/pingback`, `/ping/x` and every other path are routed as before. While Traefik shuts down gracefully, `/ping` answers `503`, so the balancer stops sending to that node before it goes away.

#### global and placement (Swarm)

| Field | Type | Description |
|-------|------|-------------|
| `global` | bool | `deploy.mode: global` on the `simpled_gateway` service: one gateway task on every node (every node matching `placement`). The default with `multi_node: true`; `false` there needs a `placement` constraint that pins the single task, and `multi_node: false` keeps the gateway on its one node. With host-mode ports, each node answers on 80 (and 443) itself, so a load balancer can target all of them. Works with `tls.letsencrypt`: the [simpled controller](#certificates-on-docker-swarm) is the only ACME client, every gateway task forwards the HTTP-01 challenges to it, and every node has the certificate secrets. |
| `placement` | object | `constraints` (list of `"<attribute> == <value>"` or `!=`) and `preferences` (list of `spread: <label>`), written to `deploy.placement` of the ingress service. `preferences` cannot be combined with `global`, so they need `global: false`. |

Both are Swarm only: set on `k8s`, `local` or standalone Docker they fail validation. The gateway publishes its ports in host mode, so it answers only on the nodes running it; global by default keeps a node loss or a reschedule from moving the one gateway away from the address DNS or the load balancer points at. Use `placement` constraints to keep it off nodes that should not take traffic.

#### Behind a load balancer (AWS ALB)

CloudFront → an internal ALB that terminates TLS with an ACM certificate → plain HTTP to Traefik on port 80 of every swarm node:

```yaml
type: docker
swarm_mode: true
controller:
  host: web
gateway:
  type: traefik
  hosts:
    web: shop.example.com
  tls:
    upstream: true
  trusted_proxies:
    - 10.0.0.0/16           # the VPC CIDR the ALB's addresses come from
  global: true
deployments:
  prod:
    # ...
    defaults:
      update:
        order: start-first
        failure_action: rollback
        monitor: 30s
      placement:
        constraints:
          - node.role == worker
```

On the AWS side: an ALB target group over every swarm node on HTTP port 80, health check path `/ping` (success code 200), and a listener on 443 with the ACM certificate forwarding to it. Keep the ALB in front of the nodes: the security group on port 80 should admit only the ALB, as the gateway itself trusts `X-Forwarded-*` from the whole `trusted_proxies` range. If CloudFront talks to the ALB over HTTP rather than HTTPS, the `X-Forwarded-Proto` the apps see is that hop's, not the client's.

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
      from_json_file:
        json_file: ./secrets.json # the field named after the secret
      from_json_env:
        json_env: APP_SECRETS     # the same, from a variable holding the JSON
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
        expose:
          - "1337"         # gateway upstream port, not published on the host
      service-on-two-hosts:
        hosts:                      # instead of host/prefix/prefixes
          hostname-alias:
            prefix: /path
            strip_prefix: false
          other-alias:
            prefixes:
              "/path1":
                strip: false
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
| `secrets_env_prefix` | string | no | Reads every valueless secret from `<prefix><secret name>` in the environment, ignoring case. See [secrets_env_prefix](#secrets_env_prefix). |
| `secrets_json` | string | no | Path to a JSON document holding a field per secret. See [secrets_json](#secrets_json). |
| `secrets_aws` | string | no | AWS Secrets Manager secret holding that same document, for secrets with no source of their own. See [secrets_aws](#secrets_aws). |
| `defaults` | object | no | Default `replicas`, `resources`, and on Swarm `update` and `placement`, applied to every service that does not set its own. |
| `services` | map | no | Per-service overrides (routing, replicas, resources, variants). |
| `exclude_services` | list | no | Services this deployment does not start. Only valid for `local`. See [exclude_services](#exclude_services). |
| `networks` | list \| map | no | Docker only: the env spec's [networks](#networks) this deployment joins, with every service (list) or the named ones (map). Under `extends` a child's value replaces the base's. |
| `volumes` | map | no | Swarm only: where each named volume lives (`node`, `path`, `nfs`). Required on Swarm for every mounted volume unless `multi_node: false`. Under `extends`, a child rebinds single volumes. See [Volumes on Docker Swarm](#volumes-on-docker-swarm). |

¹ Required unless supplied by an `extends` base, or omitted on an `abstract` template.

#### extends

A deployment may inherit from another deployment in the same env spec with `extends`. This keeps shared configuration in one place when several deployments (e.g. `staging` and `prod`) differ only in a few fields.

Merge rules, applied to the base (the deployment named by `extends`) and the child (the deployment declaring it):

- **Scalar fields** (`primary_host`, `application`, `defaults`, `secrets_folder`, `exclude_services`) are taken from the base unless the child sets them, in which case the child's value replaces the base's entirely. `exclude_services` is a list but follows this rule on purpose: a full list reads as "what this deployment leaves out", and a child can bring back a service its base excluded.
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
| `secret_name:` or `secret_name: ''` | No value — go through [the fallback chain](#the-fallback-chain). Requires the deployment to configure at least one of its sources. |
| `secret_name:` + `jq: .path` | A field of the deployment's `secrets_json` or `secrets_aws` document. |
| `secret_name:` + `env: VAR_NAME` | Read from the named shell environment variable at deploy time. |
| `secret_name:` + `file: ./path` | Read from a file at deploy time. |
| `secret_name:` + `json_file: ./path.json` | The field named after the secret in a JSON file, or the one `jq: .path` selects. See [json_file and json_env](#json_file-and-json_env). |
| `secret_name:` + `json_env: VAR_NAME` | The same, from an environment variable holding the JSON document. |

```yaml
secrets:
  db_password: localpass         # inline value (local only)
  api_key:                       # load from secrets_folder/api_key
  redis_password:
    env: REDIS_PASSWORD          # read from env var
  admin_cert:
    file: ./secrets/admin.pem    # read from file
  vendor_token:
    json_file: ./vendor.json     # field .vendor_token of the document
```

##### json_file and json_env

Several secrets that belong together can share one JSON document instead of a
file or variable each. `json_file` reads it from a file, `json_env` from an
environment variable — in CI, one GitHub secret holding the whole document:

```yaml
secrets:
  vendor_ios_token:
    json_env: VENDOR_SECRETS     # field .vendor_ios_token of $VENDOR_SECRETS
  vendor_android_token:
    json_env: VENDOR_SECRETS
    jq: .android.token           # a field not named after the secret
  local_token:
    json_file: ./vendor.json     # the same, from a file
```

```yaml
# GitHub Actions
env:
  VENDOR_SECRETS: ${{ secrets.VENDOR_SECRETS }}   # {"vendor_ios_token": "...", "android": {"token": "..."}}
```

The document is read when the deployment is prepared, exactly like `env:` and
`file:` — only for the deployment being prepared, so a sibling deployment in the
same env spec never needs it — and parsed once however many secrets name it.
The filter is a field path, as for [`secrets_json`](#secrets_json). Unlike
`secrets_json` this is the secret's own source, so there is no fall-through: an
unset variable, a file that does not exist, invalid JSON, a field the document
does not have, a `null` or an empty value each stop the deployment, naming the
secret and the document. A parse error gives the line and column, never the
content. Both are valid in every environment type.

##### Whitespace and empty values

Every source but an inline value loses the **trailing** newlines of what it read — the one `echo`, an editor or the AWS CLI appends, which is not part of the credential. It matters most for a secret mounted with `variable:`: an env file cannot carry a line break at all, so without this a file written the obvious way would fail the deployment with *contains a line break*. Interior newlines are kept, so a PEM key mounted as a file arrives intact.

What is left after that must not be empty. A secret whose file is missing its value, or whose environment variable is unset or blank, stops the deployment by name instead of reaching a service as an empty credential and failing the first request that used it.

#### undockerized_environment

Path to a `.env` file whose variables are exposed to services you run outside Docker (e.g. a process started from your IDE). The values layer on top of `environment` and are written to `<service>/undockerized.env` for `local` environments only. A service that sets [`working_dir`](#working_dir) instead gets these variables as a `.env` file in its working directory.

For `local` environments, a `.env.local` file located next to `localenv.yaml` overrides these variables: any name present in both wins from `.env.local`, and names not in `undockerized_environment` are appended. This file is intended for per-developer, machine-specific overrides — keep it out of version control (e.g. add it to `.gitignore`). It has no effect on `k8s` or `docker` environments.

```bash
# .env.local — gitignored
DB_CONNECTION_STRING=Host=localhost;Port=5432;Database=myapp
```

#### ports and instances

Local only. A local stack binds real host ports: the gateway's, and every service's published `ports`. To run several copies of one stack on a machine — two worktrees, or a test run next to your own stack — name the host ports once at the top of `localenv.yaml` and write `$port(name)` wherever one appears:

```yaml
# localenv.yaml
ports:
  web: 8080
  api: 8081
  db: 5432
port_step: 3000      # optional

gateway:
  hosts:
    web: localhost:$port(web)

deployments:
  local:
    environment:
      - PUBLIC_URL=http://localhost:$port(web)
      - MACHINE_ID=dev-$instance()
    undockerized_environment:
      - DB_URL=postgres://localhost:$port(db)/app
    services:
      api:
        ports:
          - "$port(api):80"
```

`$port(name)` and `$instance()` are expanded in every string of the env spec, in the files under `application.extra` (so an extra service can publish `"$port(db):5432"`), in values read from files and `.env.local`, and in a [test suite](#testspecyaml)'s `environment` and `wait_for`. An unknown name is an error.

An **instance** is a number; instance *N* shifts every named port by `N * port_step` and runs as its own compose project:

| | instance 0 | instance N |
|---|---|---|
| ports | as written | `+ N * port_step` |
| compose project | `<app>_local` | `<app>_local_N` |
| output directory | `local_env/` | `local_env_N/` (its own `volumes/`, so its own data) |
| `container_name` | the service name | left to compose (names are global to the Docker daemon) |

The instance comes from `--instance`, then the `SIMPLED_INSTANCE` variable, then a `.simpled-instance` file next to `localenv.yaml` holding the number, then 0. The file is the convenient one for a worktree: write it once and every command there runs as that instance. Keep it out of version control.

`port_step` defaults to the smallest multiple of 100 larger than the spread of the named ports, and must be larger than that spread, so no two instances share a port. Above instance 0 every published host port and every gateway port must be a named one: a literal port would be the same in every instance, and is rejected by name. Only host ports move — an address inside the compose network (`db:5432`) stays as written.

A host-run service writes its `.env` into its `working_dir`, which every instance in one checkout shares — run instances from separate worktrees when they include host-run services.

Some things outside the stack do not follow an instance: a tunnel or OAuth redirect registered against a fixed port, and anything else keyed to the machine. `$instance()` is there to tell instances apart in such values.

#### service overrides

| Field | Type | Description |
|-------|------|-------------|
| `host` | string | Ingress host alias. Required for `public` services. |
| `hosts` | map | Several host aliases for one service, each with its own `prefix`/`prefixes`/`strip_prefix`. Mutually exclusive with `host`/`prefix`/`prefixes`/`strip_prefix`. See [hosts](#hosts). |
| `prefix` | string | URL path prefix. Required for `public` services (unless set via `export`). |
| `prefixes` | map | Multiple prefix rules, each with optional `strip: bool`. Mutually exclusive with `prefix`. |
| `strip_prefix` | bool | Whether to strip the prefix before forwarding to upstream. Default `true`. |
| `variant` | string | Image variant to use (must be declared in `appspec.yaml`). |
| `replicas` | int | Number of pod/container replicas. Overrides `defaults.replicas`. Applied on Kubernetes and Swarm; standalone Docker and local runs always start one container. |
| `resources` | object | CPU/memory requests and limits, in Kubernetes notation. Overrides `defaults.resources` as a whole. Applied on every target; see [resources](#resources) for how Docker maps them. |
| `update` | object | Swarm only. Rolling update policy. Overrides `defaults.update` as a whole. See [update and placement](#update-and-placement-swarm). |
| `placement` | object | Swarm only. Placement constraints and preferences. Overrides `defaults.placement` as a whole. See [update and placement](#update-and-placement-swarm). |
| `ports` | list | Published host ports, `"external:internal"`. The gateway routes to `external`, so it must equal the container's port; use `expose` when no host port is wanted. |
| `expose` | list | Container ports the gateway may route to **without** publishing them on the host. The first entry wins as the upstream port, ahead of `ports`. |
| `working_dir` | string | Local only. Directory of a host-run (non-dockerized) service. See [working_dir](#working_dir). |
| `depends_on` | list | Replaces the app spec's `depends_on` for this deployment. `[]` means nothing in the stack has to be up first. See [Jobs and deployment ordering](#jobs-and-deployment-ordering). |

#### resources

`requests` and `limits` each take `memory` and `cpu`, written the Kubernetes way:
memory as bytes or with a `Ki`/`Mi`/`Gi`/`Ti` (binary) or `k`/`M`/`G`/`T`
(decimal) suffix, CPU as cores (`0.5`, `2`) or millicores (`500m`). A service's
own `resources` replaces `defaults.resources` whole; a field neither sets is
left out.

```yaml
deployments:
  prod:
    defaults:
      resources:
        limits:
          memory: 512Mi        # every service without its own resources
    services:
      web:
        resources:
          requests:
            memory: 256Mi
            cpu: 250m
          limits:
            memory: 1Gi
            cpu: "2"
```

| Target | `limits` | `requests` |
|---|---|---|
| Kubernetes | `resources.limits`; an unset field gets the built-in 128Mi / 100m | `resources.requests`, same default |
| Swarm stack service | `deploy.resources.limits` (`memory`, `cpus`) | `deploy.resources.reservations` |
| Swarm job | `docker service create --limit-memory` / `--limit-cpu` | `--reserve-memory` / `--reserve-cpu` |
| Standalone Docker | `docker run --memory` / `--cpus` | `--memory-reservation`; a CPU request is ignored, `docker run` has no reservation for it |
| Local (`simpled local`) | `deploy.resources.limits` in the compose file | `deploy.resources.reservations` |

On Docker targets only what the env spec sets is applied: without `resources`
a container is unconstrained, as before. Memory is converted to Docker's binary
units (`512Mi` becomes `512m`, `1Gi` becomes `1g`), or to an exact byte count
when the quantity is not a whole binary unit (`1G` becomes `1000000000`), and
CPU to a decimal core count (`500m` becomes `0.5`). A value that is not a
quantity, or a memory request above the memory limit, fails validation.

#### update and placement (Swarm)

```yaml
deployments:
  prod:
    defaults:
      update:
        order: start-first        # start-first | stop-first
        failure_action: rollback  # rollback | pause | continue
        parallelism: 1            # tasks updated at once; 0 means all
        delay: 10s                # between batches
        monitor: 30s              # how long a new task is watched for failure
      placement:
        constraints:
          - node.role == worker
        preferences:
          - spread: node.labels.az
    services:
      api:
        update:
          order: start-first
          parallelism: 2
```

| Field | Written as |
|-------|------------|
| `update` | `deploy.update_config` of the stack service, only the fields that are set. With `failure_action: rollback` also `deploy.rollback_config` with the same `order` and `parallelism`, since Swarm's rollback defaults (stop-first, one task at a time) would otherwise apply to the rollback of a start-first update. |
| `placement` | `deploy.placement` of the stack service (`constraints`, `preferences: [{spread}]`). On a job: `docker service create --constraint` / `--placement-pref spread=...`. |

A service's own `update` or `placement` replaces the one in `defaults` whole, with no merging of fields (the same rule as `resources`); under `extends` a child's service entry or `defaults` likewise replaces the base's. `update` never applies to jobs, which are created and removed by the deploy script. `order` and `failure_action` must be one of the listed values, `delay` and `monitor` durations such as `10s`, `1m30s` or `500ms`, a constraint `<attribute> == <value>` or `!=`; anything else fails validation. Unset, nothing is written and Swarm's defaults apply. The generated version service of a deployment takes neither.

Both fields are rejected on `k8s`, `local` and standalone Docker rather than ignored: an env spec has one `type`, so a shared `extends` base never has to serve another target, and a silently dropped constraint is how a service ends up on the wrong node.

#### expose

The gateway needs to know which port a service listens on. It takes the first
`expose` entry, then a published `ports` entry, then 80.

Reach for `expose` whenever the container does not listen on 80 and you do not
want the port on the host:

```yaml
services:
  strapi:
    host: website_admin
    prefix: /
    expose:
      - "1337"        # traefik -> http://<stack>_strapi:1337, nothing published
```

`ports` would work too, but it also publishes the port on the host — and two
deployments of one app on the same server (a dev and a prod stack side by side)
would then collide on it. `expose` has no such limit. Note that a published
port's *external* number is what the gateway routes to, so `"1338:1337"` does
not redirect the gateway to 1337; it points it at a port the container never
opened.

#### hosts

`host` puts a service on one alias. When that alias lists several domains, the
service gets the *same* prefixes on all of them. `hosts` is for the other case: one
service answering on two hosts under **different** paths.

```yaml
services:
  strapi:
    hosts:
      website_admin:        # the CMS's own domain: everything
        prefix: /
        strip_prefix: false
      website:              # the site's domain: only the media paths, so an
        prefixes:           # <img> URL stays same-origin with the page
          "/upload":
            strip: false
          "/api/translations":
            strip: false
  nextjs:
    host: website           # the single-host form is unchanged
    prefix: /
```

Each key is an alias from `gateway.hosts`, and each value takes the same
`prefix` / `prefixes` / `strip_prefix` fields a service override takes. A
service must claim at least one prefix on every alias it names — an alias with
no prefix would not be reachable, and is rejected rather than ignored.

Setting `hosts` together with any of `host`, `prefix`, `prefixes` or
`strip_prefix` is an error: the two spellings disagree about which host a
prefix belongs to.

Routes are merged per domain across every service and deployment, and two
services claiming the same path on the same domain is still an error (see
[gateway](#gateway)). Longer prefixes win at request time, so a service holding
`/` on a domain does not shadow another holding `/upload` there.

`relative` environment variables are unaffected: they resolve against the
deployment's `primary_host`, never against a service's own routing, which with
several routes could not name a single base URL.

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

A service the gateway does not route to needs no `host`, `prefix` or `ports` — `working_dir` alone is enough, e.g. a migration job or a worker you run from the IDE:

```yaml
      migrate:
        working_dir: ../migrate
```

`working_dir` is not valid for `k8s` or `docker` environments.

Two services in the same deployment cannot share a `working_dir`: each one writes its own `.env` and secrets there, so they would overwrite each other. `simpled` rejects the spec with an error naming both services. Paths are compared after normalization, so `./api`, `api` and `api/` all count as the same directory. Different deployments may reuse a directory, since only one runs at a time.

#### exclude_services

Local only. Services (app or extra) this deployment does not start. What `simpled local run --exclude` does from the command line, declared once in the spec, so that "infrastructure only" or "everything but the SPAs" is a deployment you name rather than a list of flags you repeat:

```yaml
# localenv.yaml
deployments:
  local:
    primary_host: web
    application:
      name: myapp
    services:
      api:
        host: web
        prefix: /api
        ports:
          - "4001:80"
        working_dir: ../api
      web:
        host: web
        prefix: /
        ports:
          - "4000:80"
  infra:
    extends: local
    exclude_services: [api, web]   # gateway + extra services only
```

An excluded service keeps everything except its container: its gateway routes still point at its host port, and a `working_dir` still receives its `.env` and secrets, since exclusion is how a service is handed to the developer to run by hand. Every name must be a service of the application; an unknown one is rejected, as is a name listed twice. `--exclude` on the command line adds to the list.

Under [extends](#extends) the child's list replaces the base's. `exclude_services` is not valid for `k8s` or `docker` environments.

#### networks

Docker only, Swarm and standalone. Every deployment runs on a network of its own, `simpled-<deployment>`, and reaches no other deployment. A service there answers to its bare name (`db:5432`) and to `<deployment>_<service>`; the gateway joins every deployment's network and routes to the second.

To let deployments reach each other, declare a network at the top level and list it under each deployment that joins it:

```yaml
networks:
  app1-db:
  app2-db:

deployments:
  prod_db:
    networks:
      app1-db: [postgres]          # only these services join
      app2-db: [postgres, redis]
  prod_app1:
    networks: [app1-db]            # every service joins, jobs included
  prod_app2:
    networks: [app2-db]
```

`prod_app1` and `prod_app2` reach `prod_db`, but not each other: everything on a network reaches everything else on it, so a hub serving several deployments that must stay apart needs a network per pair. A network named in a map with an empty value takes every service, like the list form.

Address another deployment's service by `<deployment>_<service>` (`prod_db_postgres:5432`), the name it has on a shared network. A bare name always resolves inside the asking deployment first, but on Swarm `docker stack deploy` also puts the bare name on every network a service joins, so a bare name the deployment does not have itself can resolve to a service across a shared network. Standalone Docker gives the bare name on the own network only.

Joining a network is part of the deployment, so a deployment added to a network is connected on its own next deploy, and so is every deployment already on it: adding `prod_app3` means redeploying `prod_db` too.

A declared network is `simpled_<name>` in Docker, created by the first deploy that needs it, as is every deployment's own network. Errors: a network no deployment or only one deployment joins, a deployment joining an undeclared network, a listed service the application does not have, a name other than letters, digits, `_`, `.` and `-`, and the reserved `control`. `networks` is not valid for `k8s` or `local` environments.

From simpled 2.4 on, a deployment leaves the shared `common_network` of earlier versions on its next deploy. The gateway stays on `common_network`, so a deployment not yet redeployed by 2.4 is still routed, but two deployments that called each other over it must now share a declared network. On standalone Docker, containers are named `<deployment>_<service>` instead of `<service>`; the old container is removed by the first deploy.

---

## testspec.yaml

The suites `simpled test` runs against a local deployment. Optional; only projects with a `localenv.yaml` and something to run against it need one.

```yaml
suites:
  suite-name:
    deployment: deployment-name    # or a deployment block, see below
    working_dir: ./e2e
    environment:
      - VAR_FROM_DEPLOYMENT
      - SUITE_VAR=${VAR_FROM_DEPLOYMENT}/api
      - $all
    secrets:
      - db_password:
          variable: PGPASSWORD
      - api_key                    # a file at working_dir/secrets/api_key
    wait_for:
      - http://localhost:4080/api/health
    timeout: 2m
    run: npm test
```

### Suite fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `deployment` | string \| object | no | The deployment in `localenv.yaml` to run against, or a deployment block of the suite's own. Omitted, the env spec's only deployment is used. See [deployment](#deployment). |
| `working_dir` | string | no | Directory the suite runs from, relative to `testspec.yaml`. Its `.env` and secrets are written there, like a host-run service's. Default: the spec's directory. |
| `environment` | list | no | The [service `environment` grammar](#service-fields): a bare name forwards the deployment's value, `NAME=value` sets one (with `${VAR}` substituted from the deployment), `$all` forwards everything. |
| `secrets` | list | no | The [service `secrets` grammar](#secret-mount-options): `variable:` goes into `.env`, `path:` or a bare name is written as a file under `working_dir`. Every name must be a secret the deployment provides. |
| `wait_for` | list | no | URLs polled after the stack is up until each answers with any status below 500. |
| `timeout` | string | no | Compose-style duration for the whole start: every `docker compose up --wait` phase, every job, and `wait_for`. Default `120s`. |
| `run` | string | yes | The command, run through `sh -c` (or `cmd /C` on Windows) in `working_dir`. Its exit status is the suite's. |

Variables are taken from the deployment as a process on this machine sees it: `environment` with `undockerized_environment` applied on top, `.env.local` included, and `$secret(...)` expanded. Forwarding a name the deployment does not define is an error naming the suite and the deployment, as is a `${VAR}` reference to one. A variable that `undockerized_environment` also sets must be overridden there as well, or the containers and the suite see different values.

The suite's variables are written to `working_dir/.env` for its own tooling and set on the process that runs `run`.

### deployment

A string names a deployment in `localenv.yaml`. A block declares one, with every field a `localenv.yaml` deployment takes, and is registered under the suite's name, so it must not collide with an existing deployment and cannot be `abstract`. The usual shape extends the everyday deployment and changes what the tests need:

```yaml
suites:
  e2e:
    deployment:
      extends: local
      exclude_services: [admin, frontend, storefront]
      environment:
        - NODE_ENV=test
      secrets:
        firebase_service_account:
          file: ../secrets/firebase_service_account_e2e
    working_dir: ./e2e
    run: npm test
```

Because it is an ordinary deployment, the [extends](#extends) merge rules apply unchanged, and the block can be moved into `localenv.yaml` verbatim when it stops being test-only.

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
  --app-version, --version <VERSION>   Version to download, with --download-bundle-from
  --download-bundle-from <SOURCE>      Download bundle: github-release
  --github-repo <OWNER/REPO>           GitHub repository
  --github-tag-prefix <PREFIX>         Prefix for GitHub release tag
  --target-binary <PATH>               Swarm: the simpled binary to ship for the deploy
                                       target (also SIMPLED_TARGET_BINARY)
  --aws-secrets-file <PATH>            Read `aws` secrets from this file and write them
                                       into the output; see aws-secrets.local.json
```

Must be run from the directory containing `envspec.yaml`.

On Docker Swarm the output includes the simpled binary that runs the deploy steps
on the target; see [Deploy steps on Docker Swarm](#deploy-steps-on-docker-swarm).

Every relative path inside the spec (`environment: file.env`, `file:` secrets, `configs`,
`secrets_folder`, `application.extra`) is resolved against the directory that holds the
spec, not against the current directory. The same holds for `simpled local --path`.

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
  --bind <ADDR>            Address the gateway listens on (default: 127.0.0.1).
                           Pass 0.0.0.0 to reach it from another device.
  --instance <N>           Run as instance N (see ports and instances)
  --app-bundle <PATH>      Run a released bundle (.tar.gz or directory): app
                           services take its version's published images
  --app-version <VERSION>  With --download-bundle-from: the version to download
  --download-bundle-from <SOURCE>, --github-repo <OWNER/REPO>,
  --github-tag-prefix <PREFIX>
                           As for prepare-deployment
```

See [Running a released bundle locally](#running-a-released-bundle-locally).

`--exclude` adds to the deployment's own [`exclude_services`](#exclude_services),
and every name must be a service of the application.

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
  --bind <ADDR>        Address the gateway listens on (default: 127.0.0.1)
  --instance <N>       Run as instance N (see ports and instances)
```

### `simpled local generate-config`

Writes `local_env/docker-compose.yaml` and per-service `.env` files without starting the gateway or running Docker Compose.

```
simpled local generate-config [OPTIONS]

Options:
  --path <PATH>        Path to the project directory (default: current dir)
  --deployment <NAME>  Deployment to generate config for. Required when the env
                       spec defines more than one deployment.
  --instance <N>       Generate for instance N (see ports and instances)
  --app-bundle, --app-version, --download-bundle-from, --github-repo,
  --github-tag-prefix  Generate for a released bundle, as `local run` does
```

### `simpled test`

Brings a local deployment up, runs a suite from [testspec.yaml](#testspecyaml) against it, tears it down, and exits with the suite's status.

```
simpled test [SUITE] [OPTIONS]

Options:
  --path <PATH>        Path to the project directory (default: current dir)
  --keep               Leave the stack running after the suite, to debug a failure
  --no-up              Run the suite against whatever is already up: no compose,
                       no gateway, no readiness wait
  --logs               Print the services' logs after every suite, not only a
                       failing one
  --instance <N|auto>  Run as instance N, or `auto`: the lowest instance whose
                       named ports are all free (not with --no-up)
  --app-bundle, --app-version, --download-bundle-from, --github-repo,
  --github-tag-prefix  Run the suite against a released bundle's published
                       images (see Running a released bundle locally)
```

Without `SUITE` every suite runs, in name order, and the exit code is the first failing suite's. One run, in order:

1. The suite's deployment is validated and resolved like `local run` would, and the suite's own variables and secrets are checked against it.
2. `working_dir/.env` and the suite's secret files are written.
3. The compose file goes to `test_env/`, as project `<application.name>_test` (`test_env_N/` and `<application.name>_test_N` for instance N), with the named volumes as Docker volumes. The developer's `local_env/` stack and its data are never touched, and a stopped one does not block the run.
4. The gateway is bound. A port held by a forgotten `local run` fails here, before anything is started.
5. The stack starts in the phases a deploy uses, all within `timeout`: the services the jobs depend on with `docker compose up --wait`, then each job to completion (a job that exits non-zero fails the suite with its exit code), then the remaining services with `--wait`. Then each `wait_for` URL is polled.
6. `run` executes in `working_dir` with the suite's variables in its environment.
7. The services' logs are printed when the suite failed (or always, with `--logs`), then `docker compose down --volumes` removes the containers and the data. Ctrl-C during the run does the same before exiting.

`--instance auto` lets suites from several worktrees run at once: each takes the first instance whose ports nothing holds. Two runs that start at the same moment can still pick the same one; the loser fails when it binds the gateway, before anything is started.

`--keep` skips the teardown and prints the `docker compose down` command to run later. `--no-up` skips steps 3 to 5 and 7 entirely, for a developer who already has the backend running from an IDE.

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

### `simpled status`

Every deployment's version, health and services, from the
[controller's private API](#private-api). Run it next to the env spec: the API's
address comes from `controller.host`, the token from `SIMPLED_TOKEN` (the read
or the manage token) or, when that is not set, from the env spec's
`controller.read_token` (else `manage_token`), read as `prepare-deployment` reads it.

An `aws:` token is fetched only on the deploy target, so here it is read like a
local deployment's: from [`aws-secrets.local.json`](#aws-secretslocaljson) next to
the env spec, else with the AWS CLI. One file per env directory, so
`cd deployment/dev && simpled status` has everything it needs, the controller's
address from the env spec and the token from the file:

```json
{
  "dev/simpled-controller": { "controller_read_token": "..." }
}
```

Put only the read token there unless you need the manage one. When the read token
cannot be read here, the manage token is tried. A refused token (401, 403) names
where it came from. With `--url` and `SIMPLED_TOKEN` no env spec is read.

```
simpled status [--url <API>] [--json]

  --url   The API's base instead, e.g. https://ops.example.com/simpled/api/v1
  --json  Print the API's answer as it is
```

```
$ SIMPLED_TOKEN=... simpled status
prod  shop 1.2.3  healthy
  api                      2/2
  primary-db               1/1
blog  blog 0.9.0  degraded, updating
  web                      1/2
```

### `simpled logs`

A service's log, from the controller: on Swarm, the lines of every task, read from the
manager. Same address and token as `simpled status`.

```
simpled logs <DEPLOYMENT> <SERVICE> [-f] [--tail <N|all>] [--since <TIME>] [--timestamps] [--url <API>]

  -f, --follow    Keep printing new lines until interrupted
  --tail          Lines from the end of each task's log (default 200)
  --since         30s, 10m, 2h, 1d, or Unix seconds
  --timestamps    Prefix every line with its time
```

### `simpled wait`

Waits until a deployment reports a version as healthy and converged: it polls the
public [`/simpled/health/<deployment>`](#health-and-versions-public) on the controller's
host, so it needs no token. For CI, right after `deploy.sh`; it fails when the time
is up, with the last state it saw.

```
simpled wait <DEPLOYMENT> --version <VERSION> [--timeout 10m] [--interval 5s] [--url <HEALTH URL>] [--insecure]
```

The services decide: a deployment that is `degraded` only by its certificate (see
`services_status` in [the health endpoint](#health-and-versions-public)) counts as done, and
the certificate's state is printed as a warning. `--insecure` accepts any
certificate, for a Let's Encrypt staging or private ACME server.

A deployment the controller does not know (its 404, `no such deployment`) fails the
wait once three polls in a row say so: waiting would not change it, but right after a
deploy that added the deployment the first poll may still reach a controller with the
config from before. Any other answer, including the gateway's own 404 before it routes
the controller right after a deploy, is waited out.

### `simpled tls import`

Turns the certificates in a Traefik `acme.json` into the Swarm gateway's certificate
secrets, so the [controller](#certificates-on-docker-swarm) does not request them again.
Run it on a swarm manager, with the Docker CLI on `PATH`.

```
simpled tls import <FILE>
```

Each name a certificate covers becomes its own secret, labelled like the ones the
controller issues. Expired certificates and wildcard names are skipped, and a certificate
imported before is not created twice. `deploy.sh` does the same on its own for a 2.0.x
`letsencrypt/acme.json` next to it.

---

## Generated output

### Kubernetes (`type: k8s`)

Output directory: `manifests/`

| File | Description |
|------|-------------|
| `deployment-<service>.yaml` | Kubernetes Deployment, one per long-running service |
| `job-<service>.yaml` | Kubernetes Job, one per `type: job` service |
| `service-<service>.yaml` | Kubernetes Service, for every long-running service that declares `ports` |
| `ingress.yaml` | Ingress resource with all routing rules |
| `configmap-<name>.yaml` | ConfigMap for each config group |
| `secret-<name>.yaml` | Secret for each secret |
| `cluster-issuer.yaml` | Let's Encrypt ClusterIssuer (if configured) |

Object names follow Kubernetes' DNS-1123 rules, so a spec name such as `db_password` is
written as `shop-db-password`: lower-cased, with every character other than letters,
digits and `.` turned into `-`. References are normalized the same way, so manifests
always agree with each other.

### Docker standalone (`type: docker`, no swarm)

Output directory: `docker-deploy/<deployment-name>/`

| File/Dir | Description |
|----------|-------------|
| `deploy.sh` | Script to pull images and start containers |
| `envs/<service>.env` | Per-service environment variable files |
| `configs/` | Configuration files |
| `secrets/` | Secret files |
| `traefik/`, `controller/controller.json` | The gateway's files and the [simpled controller](#the-simpled-controller-docker)'s config |

### Docker Swarm (`type: docker`, `swarm_mode: true`)

Output directory: `docker-deploy/`

| File | Description |
|------|-------------|
| `deploy.sh` | Deploy script: image prune, Swarm configs and secrets, ingress, then the job phases, then the stack, then pruning of old configs and secrets. The steps marked `./simpled step` are run by the binary next to it. Images no container uses are pruned *before* the rollout, so the release that is running at that point stays on the node as the rollback image. The full stack file is deployed with `--prune`, so services removed from the spec are removed from the swarm on the next deploy, and with `--detach=false`, so the script only returns once every task is running (and healthy, where a `healthcheck` is declared) |
| `simpled` | The simpled binary that runs the deploy steps, the version that prepared the deployment |
| `gateway/plan.json`, `<deployment>/plan.json` | What the deploy steps act on: the gateway's and the deployment's objects, and the deployment's jobs |
| `<deployment>/docker-compose.yaml` | Stack file with all long-running services (jobs are not part of it) |
| `<deployment>/docker-compose.deps.yaml` | Stack file with only the services the jobs depend on (phase 1); written only when that is a strict subset |
| `<deployment>/<service>/` | Per-service `.env` file, read by `docker stack deploy` on the machine that runs it |
| `<deployment>/configs/`, `<deployment>/secrets/` | The files the deployment's Swarm configs and secrets are created from. See [Configs and secrets on Docker Swarm](#configs-and-secrets-on-docker-swarm) |
| `<deployment>/volumes/<name>/` | Only for unbound volumes, which `multi_node: false` allows |
| `gateway/` | The `simpled` stack: the gateway (`simpled_gateway`, Traefik) and the [simpled controller](#the-simpled-controller-docker) (`simpled_controller`). Its `traefik/` files and `controller/controller.json` are uploaded as Swarm configs too |

The `simpled` stack publishes 80 (and 443 with TLS) in **host mode** (`mode: host` in the long port syntax), not through the routing mesh: the mesh rewrites the source address, so every client would reach the apps with the same `X-Forwarded-For` and share one per-IP rate limit. The consequence is that the gateway answers only on the node(s) running its task, so DNS or the CDN origin must point at that node, and only one gateway task per node can bind the ports. The stack keeps the default `stop-first` update order, as a second task could not bind them while the first holds them, so a gateway update drops connections for the moment the task restarts. With `gateway.global: true` there is one gateway task per node, so a load balancer can target every node; see [global and placement](#global-and-placement-swarm).

### Deploy steps on Docker Swarm

The Swarm `deploy.sh` stays the readable outline of a deploy: its order, its phases
and every `docker stack deploy` are in it. The parts that would be a lot of shell are
calls to the simpled binary shipped next to it:

| Step | Does |
|------|------|
| `./simpled step lock <deployment>` | Takes the swarm's deploy lock, waiting while another deploy holds it. `deploy.sh` releases it with `./simpled step unlock` when it exits. See [Several deployments in one env spec](#several-deployments-in-one-env-spec). |
| `./simpled step check-nodes` | With `multi_node: false` only: stops unless exactly one node accepts tasks, and exports its ID for the `node.id` constraint. See [multi_node](#multi_node). |
| `./simpled step sync-objects <plan>` | Creates the plan's configs and secrets that do not exist yet and exports their names. See [Configs and secrets on Docker Swarm](#configs-and-secrets-on-docker-swarm). |
| `./simpled step run-job <plan> <job>` | Runs a job as a `replicated-job` service, waits until it completes or fails, prints its logs and removes the service. `JOB_TIMEOUT` (seconds, default 600) limits the wait. |
| `./simpled step check-gateway <plan>` | Before the gateway is redeployed: refuses to replace a newer simpled's gateway, and prints how its routing changes. |
| `./simpled step prune-objects <plan>` | Removes configs and secrets older than the previous deploy. |

A step that sets variables for the rest of the script prints `export` lines, which
`deploy.sh` captures and then evaluates, so a failing step stops the script.

Steps read only the plans `prepare-deployment` wrote, `gateway/plan.json` and
`<deployment>/plan.json`, never the env or app spec: those depend on the CI
environment, on files next to the spec and on the downloaded bundle. Every string a
plan passes to Docker is in compose's interpolation syntax (`${NAME}` is a variable
the script exported, `$$` a literal `$`), the same as the stack files, so a value
reaches a job exactly as it reaches a stack service. A plan holds no more than the
stack file does. A step refuses a plan prepared by another simpled version.

The binary is the version that prepared the deployment, taken from, in order:

1. `--target-binary <path>` on `prepare-deployment`, or `SIMPLED_TARGET_BINARY`;
2. the running simpled itself, when it runs on Linux on the target's CPU, which is
   the usual CI runner;
3. that version's GitHub release, verified against the release's checksums.

`target_platform` names the target: `linux-amd64` (default) or `linux-arm64`.
Releases from 2.0.1 on include both Linux binaries. `--target-binary` is needed only
for a version without a release, such as a local build.

The same `rsync` of `docker-deploy/` that carries the stack files carries the
binary, and `deploy.sh` makes it executable. The target needs nothing beyond Docker
and, for `aws` secrets, the AWS CLI.

---

### Several deployments in one env spec

Every deployment of an env spec is deployed on its own, into its own stack, and its
`deploy.sh` also redeploys the shared gateway from its copy of the env spec. What
keeps them from getting in each other's way:

- **One directory per deployment on the target**, for example
  `rsync -a docker-deploy/ host:~/deploy/shop/`. Nothing generated is then shared
  between two deployments: each has its own `deploy.sh`, binary, plans and
  `gateway/` copy.
- **Networks are per deployment.** Each deployment's services are on its own
  network and reach another deployment only over a [network](#networks) both
  join. The gateway is on every deployment's network.
- **Objects have owners.** A deployment's configs and secrets are labelled
  `simpled.deployment=<name>` and the gateway's `simpled.scope=environment`, and
  pruning only touches the owner being deployed.
- **One deploy at a time per swarm.** `./simpled step lock <deployment>` takes the
  swarm's deploy lock, the Swarm config `simpled_deploy_lock`, before anything
  changes, and `deploy.sh` releases it when it exits, also on failure. A second
  deploy waits for it, up to `SIMPLED_LOCK_WAIT` seconds (default 600), then fails,
  naming the deployment and host that hold it. When the lock is older than 30
  minutes, the error says that deploy probably died; if nothing is deploying, clear
  it with `./simpled step unlock --force`.
- **The gateway is checked before it is replaced.** `./simpled step check-gateway`
  reads the labels of the running gateway (`simpled.version`,
  `simpled.deployment`):
  - It stops a deploy that would replace a gateway from a newer simpled with an
    older one. Prepare the deployment with the newer simpled, or run
    `SIMPLED_ALLOW_DOWNGRADE=1 ./deploy.sh` to do it anyway.
  - When the gateway's routing changes, it prints the changed lines of
    `dynamic_conf.yml` against the one that runs, and which deployment deployed
    that. A deploy from an older checkout of the env spec shows up as routes it
    removes.

Not handled: a deployment removed from the env spec keeps its stack running.
The gateway stops routing to it; remove it with `docker stack rm <deployment>`.

---

### Configs and secrets on Docker Swarm

On Swarm, every config file, every secret mounted as a file and the gateway's own
files (`traefik.yml`, `dynamic_conf.yml`) are native Swarm configs and secrets, not
bind mounts of the deploy directory. Swarm keeps them in its Raft store, so every
node can run a task that mounts them, not only the node that ran `deploy.sh`.
Secrets mounted as environment variables are unchanged.

Swarm objects cannot be changed, so each one is named after its content:

1. `prepare-deployment` writes the files to upload: `<deployment>/configs/<config>/<file>`
   and `<deployment>/secrets/<secret>`. A secret with an `aws` source is written there
   by `fetch-secrets.sh` on the target.
2. Before deploying anything, `./simpled step sync-objects` hashes each file and creates the object
   `<base>_<first 12 hex digits of its sha256>` unless it exists already, for example
   `prod_shop-data_settings.json_3f9a1c2e4b5d`. The base is `<deployment>_<config>_<file>`
   or `<deployment>_<secret>`, and `gateway_<file>` for the gateway. The name is
   exported in a variable (`SIMPLED_OBJECT_<n>`, `SIMPLED_GATEWAY_OBJECT_<n>`).
3. The stack files declare each object `external: true` with `name: ${SIMPLED_OBJECT_<n>}`,
   which `docker stack deploy` interpolates, and services mount it at the path the app
   spec gives, with mode `0444`. A config mounted as a directory becomes one object per
   file, mounted at `<mount path>/<file>`. A job gets `--config` and `--secret` flags
   naming the same objects.
4. The same content gives the same name, so a deploy that changes nothing creates
   nothing. New content is a new object, which changes the services that mount it,
   and Swarm does a rolling update of exactly those. Nothing time-dependent is
   written into a Swarm service, so a deploy restarts only the services whose image,
   environment, configs or secrets changed. The gateway restarts when its routing
   changes, since that renames its config. One exception restarts on every deploy:
   the deployment's version service, whose answer includes the time of the deploy.
   To restart a service without changing anything, use
   `docker service update --force <stack>_<service>`. The deploy log still says
   `Updating service ...` for every service: `docker stack deploy` sends each one an
   update, and Swarm only replaces the tasks of those whose spec differs.
5. After every stack has deployed, `deploy.sh` removes older objects: of each base
   name it keeps the two newest, the current one and the previous one, which
   `docker service rollback` still needs. Docker refuses to remove an object a
   service still uses, so that one stays too. Pruning only touches objects labelled
   with the deployment being deployed (`simpled.deployment=<name>`), or, for the
   gateway's files, `simpled.scope=environment`.

A config file or secret larger than 500 KB, Swarm's limit for one object, fails
`prepare-deployment`.

#### Certificates on Docker Swarm

With `tls.letsencrypt`, the [simpled controller](#the-simpled-controller-docker) is the
gateway's ACME client, and every domain's certificate is a Swarm secret, which every
node has. Traefik runs no ACME and keeps no `acme.json`: it reads each domain's
certificate and key, in one PEM, from `/run/secrets/simpled_tls/<domain>.pem`.

1. **First deploy.** `./simpled step tls-secrets` finds no certificate for a domain
   and creates a self-signed placeholder, so the gateway can start. Browsers warn
   until the real certificate is there, usually within a minute.
2. **Issuing.** The controller requests one certificate per domain, so a domain whose
   DNS does not point at the gateway yet fails alone. It answers Let's Encrypt's
   HTTP-01 challenge itself: every gateway task, on every node, routes
   `/.well-known/acme-challenge/` on port 80 to it, ahead of the redirect to HTTPS.
3. **Rolling out.** Each certificate becomes a secret `tls_<domain>_<hash>`, labelled
   `simpled.tls.domain`, `simpled.tls.issuer=acme` and `simpled.tls.not_after`. One
   rolling update of the gateway, with its own update policy, moves it onto all the
   certificates issued in that round.
4. **Redeploys** keep the current certificate: `step tls-secrets` picks the issued one
   that lasts longest, so the stack does not change and the gateway does not restart.
5. **Renewal.** The controller checks every hour and renews a certificate 30 days before
   it expires. A failed request is retried after 2 minutes, then 4, 8, 16 and 32,
   then after an hour, doubling up to a day: the first failures are usually a gateway
   that is not routing the challenges yet, right after a deploy. The gateway is not
   updated while a deploy holds the deploy lock; the controller tries again 5 minutes
   later. A lock older than 30 minutes, or without the time it was taken, is one a
   crashed deploy left and does not count, as for `deploy.sh`. Once
   a domain has an issued certificate, its placeholder and all but the two newest
   certificates are removed, again not while a deploy holds the lock: `step tls-secrets`
   may have picked one of them for the stack file.

The controller keeps almost no state: what it needs to know about a certificate is
in its labels, and its ACME account lives in a Swarm config (`simpled_acme_account_*`,
named after the directory and the e-mail), so every start is the same account at the
CA and restarts do not run into Let's Encrypt's limit on new accounts. If the
controller is down, the gateway keeps serving; renewals wait, with 30 days of margin.

**The `simpled` stack.** simpled 2.0.x ran the gateway in a stack called `ingress`,
the same name as Swarm's own routing-mesh network. The gateway and the controller
now run in the `simpled` stack, as `simpled_gateway` and `simpled_controller`, and
the deploy directory has `gateway/` instead of `ingress/`. The first deploy with the
new names removes the old stack (`./simpled step retire-legacy-stack`), which takes
the gateway down for a few seconds once: the new gateway cannot bind ports 80 and 443
while the old one holds them. The new stack's images are pulled first, while the old
gateway still serves, so an image that cannot be pulled stops the deploy with the old
gateway in place. It only removes an `ingress` stack whose gateway carries simpled's
`simpled.version` label; any other one stops the deploy with what to do. On
standalone Docker the gateway container is now `simpled-gateway`, and `deploy.sh`
removes the old `traefik-ingress` one.

**Upgrading from 2.0.x.** The 2.0.x gateway kept Traefik's certificates in
`letsencrypt/acme.json` next to `deploy.sh`. When `deploy.sh` finds that file, it
imports the certificates of the gateway's domains into secrets before anything else,
then renames the file `acme.json.imported`. The gateway starts with the certificates
it already had, and the controller requests nothing until they are 30 days from expiry.
Expired certificates and wildcard names are not imported, and an entry Traefik left
half-written is skipped with its reason, not a failure. Traefik wrote the file as root,
readable by root only; when `deploy.sh` runs as a user that cannot read it or rename it,
the deploy stops and names the one command to run instead
(`sudo ./simpled step tls-import gateway/plan.json letsencrypt/acme.json`). To import a
file kept elsewhere, run [`simpled tls import`](#simpled-tls-import) on a swarm manager
first.

---

### Volumes on Docker Swarm

An app spec only names its volumes. On Swarm the env spec says where each one lives,
per deployment:

```yaml
type: docker
swarm_mode: true
multi_node: true          # the default

deployments:
  prod:
    volumes:
      pg-data:
        node: db-1                      # a Docker volume on node db-1
      uploads:
        nfs: fs-1.efs.example.com:/shop/uploads
      media:
        path: /mnt/shared/shop/media    # a host path that exists on every node
      legacy:
        node: app-1
        path: /srv/shop/legacy          # a host directory on node app-1
```

| Binding | Generated | Where its services run |
|---------|-----------|------------------------|
| `node` | A Docker volume (local driver), declared in the stack file. Swarm creates it as `<deployment>_<volume>` on the node where a task first needs it. | Pinned: `node.hostname == <node>` is added to the service's placement. |
| `node` + `path` | A bind mount of that directory. It must already exist on that node: `deploy.sh` runs elsewhere and cannot create it. | Pinned to that node. |
| `path` | A bind mount of a path that exists on every node, such as a shared mount. `deploy.sh` creates it when it is missing. | Anywhere. |
| `nfs` | A Docker volume with the local driver's NFS options. Each node mounts the export itself when a task needs it, so the nodes need no `/etc/fstab` entry. | Anywhere. |

`nfs` takes `server:/export/path`, or the long form when mount options are needed:

```yaml
uploads:
  nfs:
    address: fs-1.efs.example.com
    path: /shop/uploads
    options: nfsvers=4.1      # appended to addr=<address>
```

A service follows its volumes, so a service without volumes runs on any node. A job
that mounts a volume gets the same placement and mounts the same Docker volume
(`docker service create --mount type=volume,source=<deployment>_<volume>,...`).

Validation:

| Case | Result |
|------|--------|
| A mounted volume with no binding | Error, listing the volumes in a block ready to paste. Only with `multi_node: true`. |
| A relative bind path (`./data:/data`), in the app spec or in a deployment's service `volumes` | Error: use a config for files the service only reads, or a bound volume for data it writes. Only with `multi_node: true`. Absolute host paths such as `/var/run/docker.sock` stay allowed. |
| A binding for a volume the app does not declare | Error. |
| One service mounting volumes pinned to two different nodes | Error: no node can run it. |
| A pinned volume on a service with `update.order: start-first` | Error: for a moment two tasks would write the same directory. |
| More than one replica on a pinned volume | Warning: every replica runs on that node and shares the directory. |
| A postgres, mysql, mariadb or mongo image on `nfs` or `path` | Warning: databases on NFS are prone to corruption. |
| A binding on `k8s`, `local` or standalone Docker | Error. |

#### multi_node

| Value | Meaning |
|-------|---------|
| `true` (default) | The environment is validated as if the swarm had several nodes, even when it has one, so a spec that passes keeps working when nodes are added. Every mounted volume needs a binding, relative bind paths are rejected, and the gateway is [global](#global-and-placement-swarm) unless pinned. |
| `false` | The environment lives on one node. Unbound volumes are directories under `<deployment>/volumes/` and relative bind paths are allowed, as in simpled 1.x. |

With `multi_node: false`, `deploy.sh` first counts the nodes that accept tasks
(`Ready` and `Active` in `docker node ls`) and stops before deploying anything
unless there is exactly one. The message names the nodes. Drained or down nodes do
not count, so a new node can join drained and be activated once the spec says
`multi_node: true`. Every service, job and the gateway then carries the
constraint `node.id == ${SIMPLED_NODE_ID}`, the ID of that one node, which
`deploy.sh` exports and `docker stack deploy` interpolates. This keeps Swarm from
rescheduling a task onto a node added between two deploys. The node is not
necessarily the one running `deploy.sh`: with a drained manager and one active
worker, everything runs on the worker.

`multi_node` is only valid with `swarm_mode: true`, and `multi_node: false` cannot be
combined with `gateway.global: true`.

---

### Local (`type: local`)

Output directory: `local_env/`

The compose file sets its project name to `<application.name>_local`, so several applications can be run locally on the same machine without one taking over the other's project.

| File | Description |
|------|-------------|
| `docker-compose.yaml` | Compose file for all services |
| `<service>/.env` | Per-service environment variable file |
| `<service>/undockerized.env` | Variables for services run outside Docker (unless the service sets `working_dir`) |
| `<working_dir>/.env` | Environment and secrets for a host-run service that sets `working_dir` |

`simpled test` writes the same layout to `test_env/`, as project `<application.name>_test`, plus the suite's `.env` and secret files in its `working_dir`. Keep `local_env*/` and `test_env*/` (the instance directories included) out of version control.

---

## Upgrading to 2.0.0

2.0.0 changes what simpled generates for Docker Swarm. Kubernetes, standalone Docker
and `type: local` env specs need no changes, unless they use the nginx gateway.

| A 1.x env spec with | What 2.0.0 does | What to change |
|---|---|---|
| Swarm, a mounted named volume without a binding | Error, listing the volumes in a block ready to paste | Bind each one under `deployments.<name>.volumes`, see [Volumes on Docker Swarm](#volumes-on-docker-swarm). Or set `multi_node: false` on a single-node swarm. |
| Swarm, a relative bind path such as `./data:/data` | Error | A config for files the service only reads, a bound volume for data it writes. Or `multi_node: false`. |
| Swarm, configs or secrets | Become Swarm objects, see [Configs and secrets on Docker Swarm](#configs-and-secrets-on-docker-swarm) | Nothing, unless one is larger than 500 KB. |
| Swarm, a target that is not Linux amd64 | `prepare-deployment` ships an amd64 `simpled` next to `deploy.sh` | Set [`target_platform`](#envspecyaml--localenvyaml). The arm64 binary is in releases from 2.0.1 on; with 2.0.0, pass `--target-binary`. |
| Swarm, several deployments deployed from one directory on the target | Each `deploy.sh` replaces the other's files | One directory per deployment, see [Several deployments in one env spec](#several-deployments-in-one-env-spec). |
| `gateway.type: nginx` | Error | Remove it: Traefik is the only Docker gateway. |
| `gateway.tls.secret` on Docker | Error | `tls.letsencrypt`, or `tls.upstream: true` when TLS ends in front of the gateway. |

The first 2.0.0 deploy of a Swarm deployment replaces every service once: their
mounts move to Swarm objects, and `DEPLOY_DATE` is gone. After that, a deploy
restarts only the services that changed.

CI steps that install the latest simpled, and `simpled update`, move to 2.0.0. Pin
1.13.1 until the Swarm env specs prepare without errors.
