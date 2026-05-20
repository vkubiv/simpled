# CI/CD Integration

How to automate app bundle creation and deployment using GitHub Actions. The same patterns apply to GitLab CI, CircleCI, and other systems.

---

## Overview

The typical pipeline has two stages:

1. **Build** — triggered by a push to `main`. Builds Docker images, runs `simpled app-bundle create`, uploads the bundle to GitHub Releases.
2. **Deploy** — triggered manually or by a release event. Downloads the bundle and runs `simpled prepare-deployment`, then applies the manifests.

These stages often live in separate repositories: one for the application source, one for environment configurations.

---

## Build pipeline

> **Pin the simpled version.** Use `jaxxstorm/action-install-gh-release` with an explicit `tag` set to `v<major>.<minor>` (e.g. `v1.2`). Pinning to a major.minor lets you receive patch fixes automatically while protecting against breaking changes in a future minor release. Avoid `latest` in production pipelines — a surprise upgrade can break a deployment at the worst possible time.

### Build images and create an app bundle

```yaml
# .github/workflows/create-artifact.yml
name: Create app bundle

on:
  push:
    branches: [main]
    paths:
      - 'appspec.yaml'
      - 'src/**'

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Log in to registry
        uses: docker/login-action@v3
        with:
          registry: ${{ vars.CONTAINER_REGISTRY }}
          username: ${{ secrets.REGISTRY_USERNAME }}
          password: ${{ secrets.REGISTRY_PASSWORD }}

      - name: Build API image
        run: |
          docker build \
            -f ./api/Dockerfile \
            -t mycompany/api:latest \
            ./api

      - name: Build web image
        run: |
          docker build \
            -f ./web/Dockerfile \
            -t mycompany/web:latest \
            ./web

      - name: Install simpled
        uses: jaxxstorm/action-install-gh-release@v1.10.0
        with:
          repo: vkubiv/simpled
          tag: "v1.2"   # pin to major.minor — bump when you need new features

      - name: Create and upload app bundle
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          simpled app-bundle create \
            --registry mycompany=${{ vars.CONTAINER_REGISTRY }} \
            --push-images \
            --upload-bundle-to github-release \
            --github-repo ${{ github.repository }}
```

After this runs, a GitHub Release is created (e.g. `v1.0.52`) containing `myapp.1.0.52.tar.gz`.

### Multi-architecture builds

For ARM/AMD64 variants, use `docker buildx` with `--platform`:

```yaml
      - name: Set up QEMU
        uses: docker/setup-qemu-action@v3

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Build AMD64 image
        run: |
          docker buildx build \
            --platform linux/amd64 \
            --load \
            -t mycompany/api:latest \
            ./api

      - name: Build ARM64 image
        run: |
          docker buildx build \
            --platform linux/arm64 \
            --load \
            -t mycompany/api-arm:latest \
            ./api
```

---

## Deploy pipeline

### Deploy to Kubernetes

```yaml
# .github/workflows/deploy.yml
name: Deploy

on:
  workflow_dispatch:
    inputs:
      environment:
        description: Environment to deploy to
        required: true
        default: prod
        type: choice
        options: [prod, staging]
      version:
        description: App version to deploy
        required: true

jobs:
  deploy:
    runs-on: ubuntu-latest
    environment: ${{ inputs.environment }}
    steps:
      - name: Checkout deployment repo
        uses: actions/checkout@v4

      - name: Install simpled
        uses: jaxxstorm/action-install-gh-release@v1.10.0
        with:
          repo: vkubiv/simpled
          tag: "v1.2"   # pin to major.minor — bump when you need new features

      - name: Set up kubectl
        uses: azure/setup-kubectl@v4

      - name: Configure kubectl context
        # Example for AKS:
        uses: azure/aks-set-context@v4
        with:
          resource-group: ${{ vars.AKS_RESOURCE_GROUP }}
          cluster-name: ${{ vars.AKS_CLUSTER_NAME }}
          creds: ${{ secrets.AZURE_CREDENTIALS }}

      - name: Prepare deployment
        working-directory: environments/${{ inputs.environment }}
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          DB_PASSWORD: ${{ secrets.DB_PASSWORD }}
          REDIS_PASSWORD: ${{ secrets.REDIS_PASSWORD }}
          SENDGRID_APIKEY: ${{ secrets.SENDGRID_APIKEY }}
        run: |
          simpled prepare-deployment myapp_prod \
            --app-version ${{ inputs.version }} \
            --download-bundle-from github-release \
            --github-repo mycompany/myapp

      - name: Apply manifests
        working-directory: environments/${{ inputs.environment }}
        run: kubectl apply -f k8s/
```

### Deploy to Docker Swarm

```yaml
      - name: Prepare deployment
        working-directory: environments/prod
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          DB_PASSWORD: ${{ secrets.DB_PASSWORD }}
        run: |
          simpled prepare-deployment myapp_prod \
            --app-version ${{ inputs.version }} \
            --download-bundle-from github-release \
            --github-repo mycompany/myapp

      - name: Copy stack to server
        uses: appleboy/scp-action@v0.1.7
        with:
          host: ${{ vars.DEPLOY_HOST }}
          username: deploy
          key: ${{ secrets.DEPLOY_SSH_KEY }}
          source: environments/prod/docker-deploy/
          target: /opt/deployments/

      - name: Deploy stack
        uses: appleboy/ssh-action@v1
        with:
          host: ${{ vars.DEPLOY_HOST }}
          username: deploy
          key: ${{ secrets.DEPLOY_SSH_KEY }}
          script: |
            cd /opt/deployments/docker-deploy && sudo ./deploy.sh
```

---

## Multi-repo setup

A common pattern for larger teams is to keep application code and deployment configuration in separate repositories.

```
mycompany/myapp          ← application source, appspec.yaml
mycompany/deployments    ← envspec.yaml for all environments
```

The build job in `myapp` pushes the bundle artifact. The deploy job checks out `deployments` and runs `prepare-deployment` there.

**Build job (in `myapp` repo):**
```yaml
      - name: Push bundle to deployments repo
        env:
          DEPLOY_TOKEN: ${{ secrets.DEPLOY_REPO_TOKEN }}
        run: |
          # Create bundle locally first
          simpled app-bundle create \
            --registry mycompany=${{ vars.CONTAINER_REGISTRY }} \
            --push-images

          # Then upload artifact to deployments repo releases
          gh release create v$(simpled app-bundle version) \
            --repo mycompany/deployments \
            --title "myapp $(simpled app-bundle version)" \
            myapp.*.tar.gz
```

**Deploy job (in `deployments` repo):**
```yaml
      - name: Checkout
        uses: actions/checkout@v4  # checks out deployments repo

      - name: Download bundle
        run: |
          gh release download v${{ inputs.version }} \
            --repo mycompany/myapp \
            --pattern "myapp.*.tar.gz" \
            --dir bundles/

      - name: Prepare deployment
        working-directory: prod/
        run: |
          simpled prepare-deployment myapp_prod \
            --app-bundle ../bundles/myapp.${{ inputs.version }}.tar.gz
```

---

## Secrets management

### GitHub Actions secrets

Store deployment secrets as GitHub Actions Secrets (encrypted at rest). Reference them in workflow `env:` blocks:

```yaml
env:
  DB_PASSWORD: ${{ secrets.PROD_DB_PASSWORD }}
  REDIS_PASSWORD: ${{ secrets.PROD_REDIS_PASSWORD }}
```

In `envspec.yaml`, use `env:` source:
```yaml
secrets:
  db_password:
    env: DB_PASSWORD
  redis_password:
    env: REDIS_PASSWORD
```

### External secret managers

For AWS Secrets Manager, HashiCorp Vault, or Azure Key Vault, retrieve secrets before running `simpled`:

```yaml
      - name: Get secrets from Vault
        uses: hashicorp/vault-action@v3
        with:
          url: ${{ vars.VAULT_URL }}
          token: ${{ secrets.VAULT_TOKEN }}
          secrets: |
            secret/data/myapp/prod db_password | DB_PASSWORD;
            secret/data/myapp/prod redis_password | REDIS_PASSWORD;

      - name: Prepare deployment
        env:
          DB_PASSWORD: ${{ env.DB_PASSWORD }}
          REDIS_PASSWORD: ${{ env.REDIS_PASSWORD }}
        run: simpled prepare-deployment myapp_prod --app-bundle ...
```

---

## Complete real-world example

This is a condensed version of the clinic application build + deploy workflow.

**Build** (`create-clinic-artifact.yml`):
```yaml
name: Build clinic bundle

on:
  push:
    branches: [main]
    paths: ['clinic/appspec.yaml', 'clinic/**']

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3

      - name: Login to ACR
        uses: docker/login-action@v3
        with:
          registry: allimbacr.azurecr.io
          username: ${{ secrets.ACR_USERNAME }}
          password: ${{ secrets.ACR_PASSWORD }}

      - name: Install simpled
        uses: jaxxstorm/action-install-gh-release@v1.10.0
        with:
          repo: vkubiv/simpled
          tag: "v1.2"   # pin to major.minor — bump when you need new features

      - name: Build images
        working-directory: clinic
        run: |
          docker build -t allimb/clinic-web:latest      ./web
          docker build -t allimb/clinic-admin:latest    ./admin
          docker build -t allimb/clinic-main:latest     ./main-svc
          docker build -t allimb/clinic-sales:latest    ./sales-portal

      - name: Create bundle
        working-directory: clinic
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          simpled app-bundle create \
            --registry allimb=allimbacr.azurecr.io \
            --push-images \
            --upload-bundle-to github-release \
            --github-repo mycompany/clinic
```

**Deploy** (`deploy_clinic.yml`):
```yaml
name: Deploy clinic

on:
  workflow_dispatch:
    inputs:
      version:
        description: Clinic version
        required: true

jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout deployments
        uses: actions/checkout@v4
        with:
          repository: mycompany/deployments

      - name: Azure login
        uses: azure/login@v2
        with:
          creds: ${{ secrets.AZURE_CREDENTIALS }}

      - name: Set AKS context
        uses: azure/aks-set-context@v4
        with:
          resource-group: myapp-rg
          cluster-name: myapp-aks

      - name: Login to ACR for kubectl pull
        run: az acr login --name allimbacr

      - name: Prepare deployment
        working-directory: environments/prod
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          CLINIC_DB_PASSWORD: ${{ secrets.CLINIC_DB_PASSWORD }}
          CLINIC_REDIS_PASSWORD: ${{ secrets.CLINIC_REDIS_PASSWORD }}
          CLINIC_ADMIN_PASSWORD: ${{ secrets.CLINIC_ADMIN_PASSWORD }}
          CLINIC_SENDGRID_APIKEY: ${{ secrets.CLINIC_SENDGRID_APIKEY }}
        run: |
          simpled prepare-deployment clinic_prod \
            --app-version ${{ inputs.version }} \
            --download-bundle-from github-release \
            --github-repo mycompany/clinic

      - name: Apply manifests
        working-directory: environments/prod
        run: kubectl apply -f k8s/
```
