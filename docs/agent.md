# simpled for AI coding agents

A condensed operating manual. Read this first, then pull details with
`simpled docs <topic>` or `simpled docs search <query>`.

## Mental model

`simpled` splits a deployment into two halves that are edited in different repos and
never mix:

| | File | Answers | Lives in |
|---|---|---|---|
| **Application** | `appspec.yaml` | *what* the app is made of — services, images, the variables and secrets it needs | the app repo |
| **Environment** | `envspec.yaml` (or `localenv.yaml`) | *where* it runs — domains, TLS, registries, replicas, the values for those variables and secrets | the deploy repo, or a `local/` dir |

The app half is packaged into a versioned **app bundle** (`appspec.yaml` + image
references). The same bundle is deployed unchanged to every environment. If you find
yourself putting a domain name, a connection string, or a replica count in
`appspec.yaml`, it belongs in `envspec.yaml` instead — and vice versa for a service
list or a secret *name*.

`simpled prepare-deployment` combines the two halves and generates plain manifests
(`manifests/`, `docker-deploy/`, or `local_env/`). It is a generator, not a controller —
nothing runs in the background, and the generated files are the whole output.

## The rules that break builds

Get these right and most specs validate on the first try.

1. **`appspec.yaml` `name` must equal `deployments.<name>.application.name` in the env
   spec.** Mismatch is the most common failure.
2. **`app_services` images carry no tag** — the app `version` is appended
   automatically. **`extra_services` images must carry one** (`postgres:16`).
3. **Every `external` variable without a default must be supplied** by the deployment's
   `environment`. Use `optional:` for genuinely optional ones — `optional` entries may
   not have defaults.
4. **Every secret a service uses must be declared** in `appspec.yaml` `secrets:` (name
   only, never a value) **and** given a source in the deployment's `secrets:`.
5. **Every `public` service needs `host` + `prefix`** in the deployment's `services:`,
   or an `export:` block in `appspec.yaml` as the default. `host` is a *gateway host
   alias*, not a domain — the alias is mapped to a real domain under `gateway.hosts`.
   One alias may list several domains, which all get the same prefixes; a service that
   needs *different* paths on different domains uses `hosts:` instead (see
   `docs reference --section hosts`).
6. **Named volumes must be declared** in the top-level `volumes:` list before a service
   mounts them. Host paths (`./x`, `/x`) need no declaration.
7. **A service may only reference variables** that exist in the `environment:` block
   (or use `$all`).

## Minimal working pair

`appspec.yaml` — the app repo:

```yaml
name: myapp
version: 1.0.0

environment:
  external:
    - DB_CONNECTION_STRING
    - LOG_LEVEL=info
  relative:
    - PUBLIC_URL=/

secrets:
  db_password:

volumes:
  - pg-data

app_services:
  api:
    type: public
    image: myorg/myapp-api        # tag comes from `version`
    export:
      host: web
      prefix: /
    environment:
      - $all
    secrets:
      - db_password:
        variable: DB_PASSWORD
    depends_on:
      - postgres
  migrate:
    type: job                     # runs once, before the rest of the stack
    image: myorg/myapp-migrate
    environment:
      - DB_CONNECTION_STRING
    depends_on:
      - postgres

extra_services:
  postgres:
    type: internal
    image: postgres:16            # tag required here
    volumes:
      - pg-data:/var/lib/postgresql/data
```

`localenv.yaml` — for local development (`type: local` is the default in this file):

```yaml
gateway:
  hosts:
    web: localhost:8080

deployments:
  myapp_local:
    primary_host: web
    application:
      name: myapp                 # == appspec name
      version: ^1.0.0
    secrets_folder: ./secrets     # local only
    secrets:
      db_password:                # value read from ./secrets/db_password
    environment:
      - DB_CONNECTION_STRING=Host=postgres;Database=myapp;Username=postgres;Password=$secret(db_password)
```

`envspec.yaml` — a real environment:

```yaml
type: k8s
registry:
  myorg: registry.myorg.com
gateway:
  hosts:
    web: app.myorg.com
  tls:
    letsencrypt:
      email: ops@myorg.com

deployments:
  prod:
    primary_host: web
    application:
      name: myapp
      version: ^1.0.0
    environment: ./prod.env
    secrets:
      db_password:
        aws: prod/myapp/db        # fetched on the deploy target, not in CI
    defaults:
      replicas: 2
    services:
      api:
        host: web
        prefix: /
        replicas: 4
```

## Command sequence

```bash
# In the app repo
simpled app-bundle verify                    # validate appspec + check images exist
simpled app-bundle version                   # print the version (useful in CI)
simpled app-bundle create --registry myorg=registry.myorg.com --push-images \
  --upload-bundle-to github-release --github-repo myorg/myapp

# In the deploy repo (must contain envspec.yaml)
simpled prepare-deployment prod \
  --download-bundle-from github-release --github-repo myorg/myapp --app-version 1.0.0
# -> manifests/  or  docker-deploy/   then: kubectl apply -f manifests/   or   ./docker-deploy/deploy.sh

# Local development (dir containing localenv.yaml)
simpled local run                            # generate compose + start gateway
simpled local run --exclude api              # ...without one service
simpled local only-extra                     # gateway + extra services only; run app services yourself
simpled local generate-config                # write local_env/ without starting anything

# Tests (dir containing localenv.yaml + testspec.yaml)
simpled test                                 # up, run every suite, down; exit code = first failure
simpled test e2e --keep                      # one suite, leave the stack up to look at it
simpled test e2e --no-up                     # against a stack that is already running
```

Add `--deployment <name>` to any `local` command when the env spec defines more than
one deployment. A suite in `testspec.yaml` names its deployment itself, or declares
one inline with `extends:`.

## Choosing between the pieces

| Situation | Use |
|---|---|
| Serves HTTP to the outside | `type: public` + `host`/`prefix` |
| One service on two domains under different paths | `hosts:` on the deployment's service override |
| Container listens on a port other than 80 | `expose:` on the service override (`ports:` also publishes it) |
| Background worker, DB, cache | `type: internal` |
| Migration, one-time setup | `type: job` + `depends_on: [db]` |
| Image built and versioned with the app | `app_services` |
| Third-party image with its own version | `extra_services` |
| Value differs per environment | `environment.external` |
| Value is the same everywhere | `environment.internal` |
| Value is a URL under the deployment's own domain | `environment.relative` |
| Two deployments differ in a few fields | `extends:` on the child, `abstract: true` on the base |
| A local service you run from your IDE | `working_dir:` + `simpled local only-extra` |
| A local deployment that starts only some services | `exclude_services:` on the deployment (replaces `--exclude` flags) |
| An end-to-end suite against the local stack | `testspec.yaml` suite with `deployment: {extends: local, ...}` + `simpled test` |
| Secret already in the CI environment | `env: VAR` |
| Many secrets already in the CI environment | `secrets_env_prefix: CI_SECRET_` on the deployment, and no source per secret |
| Many secrets in one JSON document | `secrets_json: ./secrets.json` locally, `secrets_aws: prod/app/bundle` for real environments; each secret takes the field named after it |
| Secret that must not pass through CI | `aws: path/to/secret` (resolved on the deploy target) |

## Error → fix

| Error | Fix |
|---|---|
| `Deployment X expects application A, but appspec is for B` | Align `application.name` with `appspec.yaml` `name`. |
| `App version X does not satisfy deployment requirement Y` | Bump `version` in `appspec.yaml`, or widen `application.version`. |
| `Environment variables [...] required by application are not provided by deployment X` | Add them to the deployment's `environment`, give them defaults, or move them to `optional`. |
| `Secret S required by application is not provided by deployment X` | Add `S` under the deployment's `secrets:` with a source. |
| `Config C required by application is not provided by deployment X` | Map `C` to a directory under the deployment's `configs:`. |
| `Config C requires file F, but it is not provided by deployment config` | Put `F` in that directory. |
| `Deployment configures service S which is not defined in application` | Typo in the deployment's `services:` key, or the service is missing from `appspec.yaml`. |
| `Deployment D excludes service S which is not defined in application` | Typo in `exclude_services`. |
| `Suite X forwards V, which deployment D does not define` | Add `V` to the deployment's `environment` (or the suite's inline block), or set it with `V=value`. |
| `Suite X uses secret S, which deployment D does not provide` | Add `S` under the deployment's `secrets:`. |
| `Suite X declares its own deployment, but the env spec already has a deployment named X` | Rename the suite, or use `deployment: X` to reference the existing one. |
| `Service S references undefined environment variable V` | Declare `V` in `appspec.yaml` `environment:`, or use `$all`. |
| `Dependency cycle in depends_on: ...` | Break the cycle named in the message. |
| `name unknown: The repository ... does not exist` (ECR) | Let `--push-images` create it (the default), or create it yourself and pass `--no-create-repos`. |

## Gotchas

- `$secret(name)` interpolates a secret into an environment value, but **not** for an
  `aws` secret — its value does not exist until deploy time. Mount it on the service
  with `variable:` instead.
- Secrets mount at `/secrets/<name>` by default; `path:` changes the location and
  `variable:` turns it into an environment variable instead.
- A secret value keeps its interior newlines but loses the trailing ones, and must not
  be empty after that: `echo "$KEY" > secrets/api_key` is fine, an empty file is an
  error naming the secret.
- A secret with no value goes through the fallback chain, in this order and skipping
  what the deployment does not set: `secrets_folder` file, `secrets_json` field,
  `secrets_env_prefix` variable (ignoring case), `secrets_aws` field. A secret that
  sets `jq:` skips the two that hold one raw value.
- `secrets_json` and `secrets_aws` default the filter to the field named after the
  secret, so one document serves the whole list. An `aws:` written on the secret
  itself is still read whole unless it sets `jq:`.
- A `job` with no `depends_on` is treated as depending on *every* long-running service,
  which starts the whole stack before the migration. Always list its dependencies.
- Give a job's dependency a `healthcheck`; without one "ready" only means the container
  was started.
- `secrets_folder`, `working_dir`, `exclude_services`, and inline literal secret values
  are **local only** — they are rejected in `k8s` and `docker` env specs.
- `simpled test` runs in its own compose project (`test_env/`, `<app>_test`) with Docker
  volumes and removes them afterwards: every run starts from empty data, so a suite
  must seed what it needs. It never touches `local_env/`.
- A suite forwards variables from the deployment's *host* view (`undockerized_environment`
  over `environment`). Override a variable in both lists if both set it.
- `.env.local` next to `localenv.yaml` overrides `undockerized_environment` per
  developer. Keep it out of version control.
- A service override is validated strictly: an unknown field is an error, not something
  quietly dropped. A `prefixes` typo used to produce wrong routing in silence.
- Only one deployment can run locally at a time; the ones you do not pick are dropped
  before validation, so they may freely share domains and ports.

## Where to read more

```
simpled docs                      # list topics
simpled docs tutorial             # blank directory -> running deployment
simpled docs reference            # every field, every flag
simpled docs reference --outline  # section headings only
simpled docs reference --section "secrets"
simpled docs examples             # 10 annotated real-world configs
simpled docs cicd                 # GitHub Actions build + deploy
simpled docs search "named volume"
```
