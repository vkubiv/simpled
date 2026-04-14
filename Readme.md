# Simple deployment - simpled

The idea is to bridge the gap between simple deployment configurations like docker-compose and super complicated Kubernetes (k8s) setups.

A lot of projects need something more flexible than docker-compose, but they can easily drown in Kubernetes configuration complexity.

Kubernetes doesn't give you a predefined structure for your deployment, so you need to invent one yourself. This is tricky and hard to do for the first time.
To make things flexible enough, but at the same time simple to use, `simpled` borrows approaches from programming like modularity, isolation, self-verification, consistency checking, and comprehensive human-readable errors.
It also borrows the app bundle concept from mobile development.

## Core concepts

There are two core concepts: **Environment** and **Application**.

## Environment

An Environment is a space where applications live. When people think of an environment, they often mean dev, stage, or production.
But often in real life, you have multiple production environments. For example, if you are a multinational business, different countries might have different regulations, requiring separate environments.

## Application

An Application is a set of closely related and interdependent services and containers, plus a description that defines how they interconnect and depend on each other.
The description plus container images form an **Application Bundle**. Each bundle is versioned.

The application description doesn't contain environment-dependent information like the domain name of your server, database host, etc.
It resembles the Ports and Adapters concept from Hexagonal Architecture.
The same application bundle can be deployed to any environment that meets its requirements.
For example, you can deploy your `my-new-app.1.0.1` to the dev environment and test it.
Now you can deploy the same bundle to your stage or prod environments. There is no need to rebuild the bundle for a specific environment.

## Application Description

`appspec.yaml` is the main description file.
First, it specifies the name and version of the application.

```yaml
name: myapp
version: 1.3.52
```

Then, it specifies variables that need to be set depending on the environment. These variables can have default values.

```yaml
environment:
  external:
    - DB_CONNECTION_STRING
    - REDIS_CONNECTION_STRING
    - SEND_GRID_API_HOST
    - ANDROID_APP_URL="https://play.google.com/store/apps/details?id=..."
    - IOS_APP_URL="https://apps.apple.com/..."
```

`optional` works like `external` but the deployment will not fail if these variables are missing from the environment.

```yaml
environment:
  optional:
    - FEATURE_FLAG_DARK_MODE
    - DEBUG_LOGGING=false
```

In the `relative` section, a special type of variable is set. They are used to set URLs relative to the application host.
This frees you from the hassle of defining these variables independently for each environment.
However, they can still be overridden by a specific environment if needed.

```yaml
environment:
  # relative: all urls should start with /
  relative:
    - WEB_APP_LOGIN_URL="/login"
    - SELLER_PORTAL_LOGIN_URL="/sales-portal/"
    - TOKEN_LOGIN_URL="/token-login/"
    - INVITATION_URL="/universal/invitation/"
    - STRIPE_SUCCESS_URL="/payment/success"
    - STRIPE_CONNECT_RETURN_URL="/profile"
    - STRIPE_CONNECT_REFRESH_URL="/main-svc/Stripe/RefreshConnectUrl"
```

`internal` sets private variables that can be used to set up interconnections between services or common configuration.

```yaml
environment:
  internal:
    - LOGGING__LOGLEVEL__DEFAULT=Error
```

## Services

There are two ways services can be defined: `app_services` and `extra_services`.
The difference is that `app_services` images always have the same version as the version defined in `appspec.yaml`.
For `extra_services`, you use a regular image version.
Let's check the next definition:

```yaml
version: 1.0.1

app_services:
  web-app:
    type: public
    image: mycompany/web-app
  backend-svc:
    type: internal
    image: mycompany/backend-svc
  admin-panel:
    type: public
    image: mycompany/admin-panel
  admin-svc:
    type: internal
    image: mycompany/admin-svc
extra_services:
  auth-gateway:
    type: public
    image: 3rd-party/auth-gateway:26.4
```

During deployment, the following images will be pulled:

- mycompany/web-app:1.0.1
- mycompany/backend-svc:1.0.1
- mycompany/admin-panel:1.0.1
- mycompany/admin-svc:1.0.1
- 3rd-party/auth-gateway:26.4

If you try to explicitly specify a version for `app_services`, you will get a validation error.

### Service types
There are three service types:
 * **public** - exposed to the external world, accessible by an external URL. Like `https://app.mycompany.com`
 * **internal** - only accessible to other app services.
 * **job** - runs once per deployment, used to set things up. Not accessible to other services.

### Service variants

A service can define multiple image variants. This is useful when you need to deploy the same service with different base images (e.g. different architectures or flavors) and let each environment pick the appropriate one.

```yaml
app_services:
  backend-svc:
    type: internal
    image: mycompany/backend-svc
    variants:
      arm:
        image: mycompany/backend-svc-arm
```

The deployment can then select a variant with the `variant` field (see [Deployments](#deployments)).

### Service export

`export` pins a default host and prefix for a service. This is useful when the service is always expected at a fixed path regardless of environment overrides.

```yaml
app_services:
  web-app:
    type: public
    image: mycompany/web-app
    export:
      host: myapp
      prefix: /
```

### Service env variables

By default, no environment variables are passed to the service.

You can pass specific variables from the root `environment:` section:

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - DB_CONNECTION_STRING
      - REDIS_CONNECTION_STRING
      - SEND_GRID_API_HOST
```    

You can pass all variables from the root `environment:` section:

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - $all
```    

You can pass all variables and override some of them:

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - $all
      - LOGGING__LOGLEVEL__DEFAULT=Warning
```    

## Configuration files

If you need environment-dependent configuration files, you can define them in the following way:

```yaml
configs:
  # data is the name of the config
  data:
    - countries_payment_settings.json    
```

Usage in service:

```yaml
  backend-svc:
    image: mycompany/backend-svc

    # ...

    # will put countries_payment_settings.json at /data/countries_payment_settings.json path
    configs:
      - data: /data
```

## Secrets

All secrets available for the application are defined on the root element `secrets:`.

```yaml
secrets:
  redis_password:
  db_password:
  admin_password:
  sendgrid_apikey:
```

By default, secrets are mounted as files into the `/secrets/` directory.

Usage in service:

```yaml
  backend-svc:
    image: mycompany/backend-svc
  secrets:
    # mounted to `/secrets/redis_password`
    - redis_password:

    # mounted to `/custom_path/db_password`
    - db_password:
      path: `/custom_path/db_password`

    # set as SENDGRID_API_KEY environment variable
    - sendgrid_apikey:
      variable: SENDGRID_API_KEY
```

## Volumes

Named volumes that services need to persist data across restarts are declared at the application level and then referenced from individual services.

### Declaring named volumes

List volume names under the top-level `volumes:` key in `appspec.yaml`:

```yaml
volumes:
  - postgres-data
  - redis-data
  - uploads
```

### Using volumes in a service

The `volumes:` field on a service accepts entries in the standard `source:target` format.

**Named volume** — the name must be declared in the app-level `volumes:` list:

```yaml
extra_services:
  postgres:
    image: postgres:16
    volumes:
      - postgres-data:/var/lib/postgresql/data
```

**Host path** — use a relative (`./`) or absolute (`/`) path as the source; no app-level declaration is required:

```yaml
app_services:
  backend-svc:
    image: mycompany/backend-svc
    volumes:
      - ./local-data:/app/data
      - /mnt/shared:/app/shared
```

`simpled` returns an error at parse time if a service references a named volume that is not declared in `volumes:`.

## Defining environments

The Environment is defined in `envspec.yaml`.
It consists of three parts: environment type, ingress, and deployments.

### Environment type

The top-level `type` field specifies the target platform:

```yaml
type: k8s      # Kubernetes (default)
# type: docker # Docker / Docker Swarm
# type: local  # Local development
```

When `type` is `docker`, you can enable Swarm mode:

```yaml
type: docker
swarm_mode: true
```

### Registry

You can define a registry mapping at the environment level so that image names are automatically rewritten during deployment:

```yaml
registry:
  mycompany: my-docker-registry.com
```

### Ingress

Ingress defines host domain names.

```yaml
ingress:
  name: myapp-ingress
  hosts:    
    myapp: app.myapp.com
    myapp-sales: sales.myapp.com
    website: 
     - www.myapp.com
     - myapp.com
  tls:
    secret: myapp-tls
```

#### TLS options

```yaml
ingress:
  name: myapp-ingress
  hosts:
    myapp: app.myapp.com
  tls:
    # disable TLS entirely
    disable: true

    # use an existing TLS secret
    secret: myapp-tls

    # or provision a certificate automatically via Let's Encrypt
    letsencrypt:
      email: ops@mycompany.com
      # server: https://acme-staging-v02.api.letsencrypt.org/directory  # optional, defaults to production
```

When `type` is `docker`, you can also select the ingress controller:

```yaml
ingress:
  name: myapp-ingress
  type: nginx   # nginx or traefik (default)
  hosts:
    myapp: app.myapp.com
```

### Deployments

Let's define a deployment config for `myapp` web app.

```yaml
deployments:
  myapp_prod:
    # primary_host is required: the main host name (key from ingress.hosts) used to
    # build relative environment variable URLs.
    primary_host: myapp

    application:
      # application name, in this case `myapp` 
      name: myapp
      # extra: allows defining additional services that are specific for a given environment.   
      # for example on dev we can host a database inside your cluster. But on prod we can use a cloud managed database. 
      extra:
        - clinic.ext.yaml

    # spec file with environment variables defined.
    environment: clinic.env        
    # adds all files from ./data to config named data 
    configs:
      data: ./data

    # defines secrets available for the given application.
    secrets:
      redis_password:
      db_password:
      admin_password:
      sendgrid_apikey:
    
    services:
      # each public service should have `host` and `prefix` defined
      web-app:
        host: myapp
        prefix: /
      admin-panel:
        host: myapp
        prefix: /admin-panel
      auth-gateway:
        host: myapp
        prefix: /api
        # you can override default resources limitations. 
        replicas: 2
        resources:
          requests:
            memory: "128Mi"
            cpu: "500m"
          limits:
            memory: "256Mi"
            cpu: "1000m"

```

You can deploy multiple applications in the same environment. 
Let's deploy a website that contains frontend and headless CMS services.

```yaml
  website_prod:
    primary_host: website

    application:
      name: website
      # sets restriction on app version supported. In this case you can deploy any app version from 1.1.5 to 2.0.0
      version: ^1.1.5
    secrets:
      redis_password:
        env: REDIS_PASSWORD # read secret from environment variable REDIS_PASSWORD during deployment time.
      db_password:
        file: ./secrets/website_db_password.txt # read secret from a file during deployment time.
    # define default resource configuration for all services. Can be overridden for individual services. 
    defaults:
      replicas: 2
      resources:
        requests:
          memory: "128Mi"
          cpu: "500m"
        limits:
          memory: "256Mi"
          cpu: "1000m"
    services:
      website:
        host: website
        prefix: /
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

#### `undockerized_environment`

Some local environments run a mix of containerized and non-containerized services. Use `undockerized_environment` to pass variables to services that run outside Docker/Kubernetes:
This feature is 

```yaml
  myapp_prod:
    primary_host: myapp
    application:
      name: myapp
    environment: clinic.env
    undockerized_environment: clinic-native.env
```

## Exposed services

Each public service should have `host` and `prefix` defined.

* `host` is an abstract host name; it will be substituted by the real domain name from the deploying environment.
* `prefix` is the path prefix.

```yaml
services:
  myapp:
    host: myapp
    prefix: /
  admin-panel:
    host: myapp
    prefix: /admin-panel
```

In the given example, `admin-panel` will be served on the dev environment at
`https://myapp-dev.com/admin-panel`

Use `strip_prefix: false` to forward the prefix path to the upstream service without stripping it:

```yaml
services:
  admin-panel:
    host: myapp
    prefix: /admin-panel
    strip_prefix: false
```

Use `variant` to deploy a specific image variant defined in `appspec.yaml`:

```yaml
services:
  backend-svc:
    variant: arm
```

# Prepare deployment artifacts
Build all docker images for the given application:

`cd web-app && docker build -f ./Dockerfile -t mycompany/web-app:latest .`

`cd backend-svc && docker build -f ./Dockerfile -t mycompany/backend-svc:latest .`

...

Run `simpled verify`

It will check if all services from `appspec.yaml` have a docker image built for them.

`simpled app-bundle create --registry mycompany=my-docker-registry.com --push-images`

This will tag all images with the proper version from `appspec.yaml` and push them to your docker registry.
Then it will create an `appname.$version.tar.gz` artifact. E.g. `myapp.1.0.52.tag.gz`

Upload it to GitHub releases, or on S3 storage. If you use a `simpled` compatible artifact storage, you can add the `--upload` parameter:

`simpled app-bundle create --upload https://storage-domain.com/simpled`

`simpled` can create a GitHub release for you if you provide a GitHub token:

```bash 
set GITHUB_TOKEN=my-github-token

simpled app-bundle create \
  --registry mycompany=my-docker-registry.com \
  --push-images \
  --upload-bundle-to github-release \
  --github-repo mycompany/myapp
```

# Deploy application

Navigate into the folder with `envspec.yaml`. e.g. `deployments/prod`

Download the app bundle into some folder. e.g. `deployments/myapp.1.0.52.tag.gz`

Then run:

`simpled prepare-deployment myapp_prod --app-bundle deployments/myapp.1.0.52.tag.gz`

`--app-bundle` can point to a folder with `appspec.yaml` or a `tar.gz` archive of that folder.

And apply generated manifests:

`kubectl apply -f k8s/`

If you use a `simpled` compatible artifact storage, there is no need to manually download:

`set SIMPLED_API_KEY=my-api-key`

`simpled prepare-deployment myapp_prod --app-version 1.0.52 --download-bundle-from simpled-repo`

From GitHub releases:

`set GITHUB_TOKEN=my-github-token`

```bash
simpled prepare-deployment myapp_prod \
   --app-version 1.0.52 \
   --download-bundle-from github-release \
   --github-repo mycompany/myapp   
```

And apply generated manifests:

`kubectl apply -f k8s/`

If all application requirements are met in a given environment, the app will be deployed.
