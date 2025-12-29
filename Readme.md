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

By default, secrets are mounted as files into the `/etc/secrets/` directory.

Usage in service:

```yaml
  backend-svc:
    image: mycompany/backend-svc
  secrets:
    # mounted to `/etc/secrets/redis_password`
    - redis_password:

    # mounted to `/custom_path/db_password`
    - db_password:
      path: `/custom_path/db_password`

    # set as SENDGRID_API_KEY environment variable
    - sendgrid_apikey:
      environment: SENDGRID_API_KEY
```

## Defining environments

The Environment is defined in `envspec.yaml`.
It consists of two parts: ingress and deployments.
Ingress defines the root ingress, and deployments define applications that are deployed in this environment.

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
  secret: myapp-tls

```

### Deployments

Let's define a deployment config for `myapp` web app.

```yaml
deployments:
  myapp_prod:
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
    application:
      name: website
      # sets restriction on app version supported. In this case you can deploy any app version from 1.1.5 to 2.0.0
      version: ^1.1.5
    secrets:
      redis_password:
      db_password:
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
          - "/upload":
            strip: false
          - "/content-manager":
            strip: false
          - "/admin":
            strip: false
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

## Create or update secrets

You can upload files from a folder as secrets:

`simpled secrets set myapp_prod ./myapp_prod_sercrets`

You can set secrets through the command line:

`simpled secrets set myapp_prod -f redis_password="${{ secrets.REDIS_PASSWORD }}" -f db_password="${{ secrets.DB_PASSWORD }}"`

Then apply generated manifests:

`kubectl apply -f k8s/`

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
