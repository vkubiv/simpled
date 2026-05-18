# Getting Started with simpled

This tutorial walks you through deploying a real web application — a backend API with a database migration job — from zero to running locally, then packaging it for production Kubernetes.

## What you'll build

A `backend` application with:
- `api` — an HTTP API service (public)
- `web` — a public frontend
- `db-setup` — a one-time database migration job

By the end you'll have:
1. A working local environment via Docker Compose
2. A versioned app bundle ready to deploy
3. Kubernetes manifests for production

---

## Prerequisites

- `simpled` installed and on your PATH
- Docker installed and running
- `kubectl` configured (for the Kubernetes section)

---

## Step 1 — Write appspec.yaml

Create a new directory for your application and add `appspec.yaml`:

```yaml
name: backend
version: 1.0.0

environment:
  external:
    - DB_CONNECTION_STRING
    - REDIS_URL
  optional:
    - LOG_LEVEL=Information
  internal:
    - ASPNETCORE_ENVIRONMENT=Production
```

`external` variables must be provided by the environment at deploy time. `optional` ones have defaults and won't block a deployment if missing. `internal` ones are set by you and are the same across all environments.

---

## Step 2 — Define services

Add your services to `appspec.yaml`:

```yaml
app_services:
  web:
    type: public
    image: mycompany/web
    environment:
      - $all

  api:
    type: public
    image: mycompany/api
    environment:
      - DB_CONNECTION_STRING
      - REDIS_URL
      - LOG_LEVEL

  db-setup:
    type: job
    image: mycompany/api
    environment:
      - DB_CONNECTION_STRING
```

Key points:
- `app_services` automatically get the version tag from `appspec.yaml` (e.g. `mycompany/api:1.0.0`)
- `type: public` services are exposed via the ingress — use this for any service that responds to HTTP requests
- `type: internal` is for background workers, queue consumers, and support processes that do **not** serve HTTP requests
- `type: job` services run once per deployment (e.g. database migrations)
- `$all` passes every variable from the root `environment:` section to the service

---

## Step 3 — Add secrets

If your app needs credentials that shouldn't live in environment files:

```yaml
secrets:
  db_password:
  redis_password:
```

Reference them in services:

```yaml
app_services:
  api:
    type: public
    image: mycompany/api
    environment:
      - DB_CONNECTION_STRING
      - REDIS_URL
    secrets:
      - db_password:
      - redis_password:
        variable: REDIS_PASSWORD
```

By default secrets are mounted as files at `/secrets/<name>`. Use `variable:` to inject as an environment variable instead.

---

## Step 4 — Create a local environment

Create a `local/` directory and add `localenv.yaml` (the `type: local` default is implied, so the field can be omitted):

```yaml
# local/localenv.yaml
ingress:
  name: backend-ingress
  hosts:
    backend: localhost:8080

deployments:
  backend_local:
    primary_host: backend

    application:
      name: backend

    environment: backend.env

    secrets:
      db_password: localdevpassword
      redis_password: localdevpassword
    
    services:
      web:
        host: backend
        prefix: /
      api:
        host: backend
        prefix: /api
```

If you prefer to keep secret values out of the spec file, use `secrets_folder` and store each secret in a separate file:

```yaml
# local/localenv.yaml
secrets_folder: ./secrets

deployments:
  backend_local:
    ...
    secrets:
      db_password:          # reads local/secrets/db_password
      redis_password:       # reads local/secrets/redis_password
```

Now create `local/backend.env`:

```
DB_CONNECTION_STRING=Server=localhost;Database=backend_dev;User=app;Password=localdevpassword
REDIS_URL=redis://localhost:6379
LOG_LEVEL=Debug
```

---

## Step 5 — Build your images

Build images using the `latest` tag:

```bash
docker build -t mycompany/web:latest ./web
docker build -t mycompany/api:latest ./api
```

---

## Step 6 — Run locally

From the `local/` directory:

```bash
simpled local run
```

This generates `local/local_env/docker-compose.yaml` and starts all services. A reverse proxy listens on `localhost:8080` and routes requests based on your ingress rules.

simpled looks for `localenv.yaml` (or `envspec.yaml`) automatically — no extra flags needed.

- `http://localhost:8080/` → `web` service
- `http://localhost:8080/api` → `api` service

To exclude a service (e.g. run the API outside Docker for debugging):

```bash
simpled local run --exclude api
```

Check `local/local_env/backend-api/.env` for the environment variables simpled calculated for that service — you can source them before starting your process directly.

---

## Step 7 — Create an app bundle

Once your images are built and `simpled verify` passes from the app directory:

```bash
simpled app-bundle create \
  --registry mycompany=registry.mycompany.com \
  --push-images
```

This:
1. Retags `mycompany/web:latest` → `registry.mycompany.com/mycompany/web:1.0.0`
2. Pushes all images to your registry
3. Creates `backend.1.0.0.tar.gz` — the app bundle

The bundle contains only `appspec.yaml`. Images stay in the registry.

To upload to GitHub releases at the same time:

```bash
export GITHUB_TOKEN=ghp_...

simpled app-bundle create \
  --registry mycompany=registry.mycompany.com \
  --push-images \
  --upload-bundle-to github-release \
  --github-repo mycompany/backend
```

---

## Step 8 — Create a production environment

Create a `prod/` directory with `envspec.yaml`:

```yaml
type: k8s

registry:
  mycompany: registry.mycompany.com

ingress:
  name: backend-ingress
  hosts:
    backend: api.mycompany.com
  tls:
    letsencrypt:
      email: ops@mycompany.com

deployments:
  backend_prod:
    primary_host: backend

    application:
      name: backend
      version: ^1.0.0

    environment: backend.env

    secrets:
      db_password:
        env: DB_PASSWORD
      redis_password:
        env: REDIS_PASSWORD

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
      web:
        host: backend
        prefix: /
      api:
        host: backend
        prefix: /api
        replicas: 3
```

Create `prod/backend.env` with your production values:

```
DB_CONNECTION_STRING=Server=prod-db.internal;Database=backend;User=app;Password={db_password}
REDIS_URL=redis://prod-redis.internal:6379
```

---

## Step 9 — Deploy to Kubernetes

Set your secret environment variables, then generate manifests:

```bash
export DB_PASSWORD=supersecret
export REDIS_PASSWORD=anothersecret

simpled prepare-deployment backend_prod \
  --app-bundle ../backend.1.0.0.tar.gz
```

Or download the bundle directly from GitHub releases:

```bash
export GITHUB_TOKEN=ghp_...

simpled prepare-deployment backend_prod \
  --app-version 1.0.0 \
  --download-bundle-from github-release \
  --github-repo mycompany/backend
```

Both produce a `k8s/` directory. Apply it:

```bash
kubectl apply -f k8s/
```

simpled generates:
- `deployment-*.yaml` for each service
- `service-*.yaml` for each service
- `ingress.yaml` with TLS and routing rules
- `secret-*.yaml` for each secret
- `cluster-issuer.yaml` for Let's Encrypt (if configured)

---

## What's next

- [Examples](examples.md) — annotated real-world configurations
- [Reference](reference.md) — every field in appspec.yaml and envspec.yaml
- [CI/CD Integration](cicd.md) — automating builds and deployments with GitHub Actions
