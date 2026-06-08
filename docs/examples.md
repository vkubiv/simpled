# Examples

Real-world configurations showing how to use simpled across different scenarios.

---

## Example 1 — Minimal single-service app

The simplest possible setup: one service, one environment variable, local deployment only.

**`appspec.yaml`**
```yaml
name: room_scaner_backend
version: 1.0.0

environment:
  external:
    - DOWNLOAD_TEST_SIZE_BYTES=10485760
    - UPLOAD_TEST_MAX_SIZE_BYTES=107374182

app_services:
  speed-test-svc:
    type: public
    image: mycompany/speed-test-svc
    environment:
      - DOWNLOAD_TEST_SIZE_BYTES
      - UPLOAD_TEST_MAX_SIZE_BYTES
```

**`localenv.yaml`**
```yaml
gateway:
  hosts:
    backend: localhost:9080

deployments:
  local:
    primary_host: backend

    application:
      name: room_scaner_backend

    services:
      speed-test-svc:
        host: backend
        prefix: /
```

Both variables have defaults so `environment:` in the deployment can be omitted entirely. Run with:

```bash
simpled local run
```

---

## Example 2 — Multi-service app with database

A backend with an API, a background worker, an admin service, and a database setup job. This pattern is common for .NET or Node.js backends.

**`appspec.yaml`**
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

secrets:
  db_password:
  redis_password:
  admin_password:
  sendgrid_apikey:
  firebase_admin_json:

app_services:
  admin-svc:
    type: public
    image: mycompany/admin-svc
    environment:
      - DB_CONNECTION_STRING
      - LOG_LEVEL
    secrets:
      - db_password:
      - admin_password:
        variable: ADMIN_PASSWORD

  customer-svc:
    type: public
    image: mycompany/customer-svc
    environment:
      - DB_CONNECTION_STRING
      - REDIS_URL
      - LOG_LEVEL
    secrets:
      - db_password:
      - redis_password:
        variable: REDIS_PASSWORD

  db-setup:
    type: job
    image: mycompany/admin-svc
    environment:
      - DB_CONNECTION_STRING
    secrets:
      - db_password:

  notification-worker:
    type: internal
    image: mycompany/notification-worker
    environment:
      - REDIS_URL
    secrets:
      - redis_password:
      - sendgrid_apikey:
        variable: SENDGRID_API_KEY
      - firebase_admin_json:
        path: /secrets/firebase/admin.json
```

**`localenv.yaml`** with infrastructure services:

```yaml
gateway:
  hosts:
    backend: localhost:9080

deployments:
  local:
    primary_host: backend

    application:
      name: backend

    environment: backend.env

    secrets:
      db_password: localpass
      redis_password: localpass
      admin_password: localadminpass
      sendgrid_apikey: fake-key
      firebase_admin_json:
        file: ./firebase_admin.json

    services:
      admin-svc:
        host: backend
        prefix: /admin-api
      customer-svc:
        host: backend
        prefix: /api
```

Alternatively, use `secrets_folder` to keep all values out of the spec file. Create a `secrets/` directory (git-ignored), put one file per secret inside it, and leave the secret values empty:

```yaml
# localenv.yaml
deployments:
  local:
    secrets_folder: ./secrets
    ...
    secrets:
      db_password:            # reads ./secrets/db_password
      redis_password:         # reads ./secrets/redis_password
      admin_password:         # reads ./secrets/admin_password
      sendgrid_apikey:        # reads ./secrets/sendgrid_apikey
      firebase_admin_json:
        file: ./firebase_admin.json   # file source still works alongside secrets_folder
```

**`infra-services.yaml`** — extra services (database, cache, mail) for local development:

```yaml
extra_services:
  mock-mailer:
    type: internal
    image: mailhog/mailhog:v1.0.1

  primary-db:
    type: internal
    image: postgres:16.2-alpine
    environment:
      - POSTGRES_PASSWORD={db_password}
    secrets:
      - db_password:
        variable: POSTGRES_PASSWORD
    volumes:
      - postgres-data:/var/lib/postgresql/data

  redis:
    type: internal
    image: redis:7-alpine
    command: redis-server --requirepass {redis_password}
```

Reference extra services from the deployment:

```yaml
application:
  name: backend
  extra:
    - infra-services.yaml
```

---

## Example 3 — Image variants for different architectures

When you need different base images per environment (e.g. ARM vs AMD64), define variants on the service:

**`appspec.yaml`**
```yaml
app_services:
  api:
    type: public
    image: mycompany/api
    variants:
      arm:
        image: mycompany/api-arm
```

**`envspec.yaml`** on an ARM server:
```yaml
deployments:
  prod:
    services:
      api:
        variant: arm
        host: backend
        prefix: /api
```

On a standard AMD64 server, omit the `variant` field and the default image is used.

---

## Example 4 — Multiple applications in one environment

Deploy two independent applications into the same Kubernetes cluster or Docker Swarm. Common for a main product plus a lightweight companion app (admin portal, compliance tool, etc.).

**`envspec.yaml`**
```yaml
type: docker
swarm_mode: true

registry:
  allimb: allimbacr.azurecr.io

gateway:
  hosts:
    clinic: clinic.mycompany.com
    scrg: compliance.mycompany.com
    website:
      - www.mycompany.com
      - mycompany.com
  tls:
    letsencrypt:
      email: ops@mycompany.com

deployments:
  # --- compliance portal (simple, single service) ---
  scrg_prod:
    primary_host: scrg

    application:
      name: scrg
      version: ^1.0.0

    services:
      scrg-portal:
        host: scrg
        prefix: /

  # --- main clinic application ---
  clinic_prod:
    primary_host: clinic

    application:
      name: clinic
      version: ^2.3.0
      extra:
        - clinic.ext.yaml   # production-only extra services (redis cluster, etc.)

    environment: clinic.env

    secrets:
      redis_password:
        env: CLINIC_REDIS_PASSWORD
      db_password:
        env: CLINIC_DB_PASSWORD
      admin_password:
        env: CLINIC_ADMIN_PASSWORD
      sendgrid_apikey:
        env: CLINIC_SENDGRID_APIKEY

    defaults:
      replicas: 1
      resources:
        requests:
          memory: "128Mi"
          cpu: "250m"
        limits:
          memory: "512Mi"
          cpu: "1000m"

    services:
      web:
        host: clinic
        prefix: /
      admin:
        host: clinic
        prefix: /admin
      sales-portal:
        host: clinic
        prefix: /sales
      main-svc:
        host: clinic
        prefix: /api
        replicas: 2
        resources:
          requests:
            memory: "256Mi"
            cpu: "500m"
          limits:
            memory: "1Gi"
            cpu: "2000m"
```

**`clinic.ext.yaml`** — extra services loaded only in prod:

```yaml
extra_services:
  redis1:
    type: internal
    image: redis:7-alpine
    secrets:
      - redis_password:
        variable: REDIS_PASSWORD

  redis2:
    type: internal
    image: redis:7-alpine
    secrets:
      - redis_password:
        variable: REDIS_PASSWORD

  mock-mailer:
    type: internal
    image: mailhog/mailhog:v1.0.1
```

Each deployment is independent — `simpled prepare-deployment scrg_prod` and `simpled prepare-deployment clinic_prod` can run separately with their own app bundles.

---

## Example 5 — Configuration files

Use `configs` when services need environment-dependent files (JSON config, XML, certificates).

**`appspec.yaml`**
```yaml
configs:
  data:
    - country_payments.json
    - exercises.json
    - questionnaire_program.xml

app_services:
  main-svc:
    type: public
    image: mycompany/main-svc
    configs:
      - data: /app/data
```

**`envspec.yaml`**
```yaml
deployments:
  clinic_prod:
    configs:
      data: ./data   # directory containing the three files
```

At deploy time simpled reads the files from `./data/` and embeds them into a Kubernetes ConfigMap (or mounts them via Docker volumes for Docker deployments). The service receives them at `/app/data/country_payments.json`, etc.

---

## Example 6 — Multiple path prefixes for a single service

Some services (e.g. headless CMS) expose multiple distinct paths. Use `prefixes` instead of `prefix`:

```yaml
services:
  headless-cms:
    host: website
    prefixes:
      "/upload":
        strip: false
      "/content-manager":
        strip: false
      "/admin":
        strip: false
```

With `strip: false`, the prefix is forwarded to the upstream service unchanged. Without it (or `strip: true`), the prefix is stripped before forwarding.

---

## Example 7 — Pinning service exports

Use `export` to set a default host and prefix for a service in `appspec.yaml`. This means deployers don't need to repeat it every time:

```yaml
app_services:
  web-app:
    type: public
    image: mycompany/web-app
    export:
      host: myapp
      prefix: /
```

The environment's `envspec.yaml` still needs a matching `host` entry in `ingress.hosts`, but the `services:` block can omit host/prefix for this service.

---

## Example 8 — Mixed local/non-dockerized development

`undockerized_environment` generates a separate `.env` file for services you run outside Docker (e.g. your IDE debugger):

```yaml
deployments:
  backend_local:
    primary_host: backend
    application:
      name: backend
    environment: backend.env
    undockerized_environment: backend-native.env
```

**`backend-native.env`** replaces service hostnames with `localhost` equivalents so the process running on the host machine can reach the dockerized infrastructure. The file is generated at `local_env/undockerized.env`.

For `local` environments, a `.env.local` file placed next to `localenv.yaml` overrides any of these `undockerized_environment` variables (and adds new ones). Keep it out of version control so each developer can point host-run services at their own local infrastructure:

```bash
# .env.local — gitignored, per-developer overrides
DB_CONNECTION_STRING=Host=localhost;Port=5432;Database=backend
```

---

## Example 9 — Registry mapping

Map short image prefixes to full registry URLs. This keeps `appspec.yaml` registry-agnostic:

```yaml
# envspec.yaml
registry:
  mycompany: registry.mycompany.com
  allimb: allimbacr.azurecr.io
```

An image named `mycompany/api` becomes `registry.mycompany.com/mycompany/api:1.0.0` at deploy time. No changes needed to `appspec.yaml` when switching registries between environments.

---

## Example 10 — Named volumes

Declare volumes at the app level, then reference them from services:

**`appspec.yaml`**
```yaml
volumes:
  - postgres-data
  - redis-data
  - uploads

extra_services:
  postgres:
    image: postgres:16
    volumes:
      - postgres-data:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    volumes:
      - redis-data:/data

app_services:
  api:
    type: public
    image: mycompany/api
    volumes:
      - uploads:/app/uploads
      - ./local-seed:/app/seed   # host path, no declaration needed
```

simpled returns a parse error if a service references a named volume not declared in `volumes:`.
