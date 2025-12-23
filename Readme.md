# Simple deployment - simpled

The idea is too bridge the gap between simple deployments configuration like docker-compose, and
super complicated k8s.

A lot of project need something more flexible then docker-compose, but they will drown in k8s
configuration complexity.

K8s didn't give you predefined structure for you deployment, so you need to invent some by your
self.
This is tricky and hard to do for the first time.
To make things flexible enough, but in the same time simple to use, `simpled` borrow approaches from
programing like modularity, isolation, self verification, consistency checking, comprehensive human
readable errors.
Also it borrows app bundle concept from mobile development.

## Core concepts

There two core concept **Environment** and **Application**

## Environment

Is space where applications live. Then people think environment they often mean dev, stage,
production.
But often in the real life you have multiple production environments. For example you are
multinational business,
and different countries have different regulations.

## Application

Application is set of closely related and interdependent services, containers.
Plus a description that defines how they interconnect and depends on each other.
The description plus container images forms **Application Bundle**. Each bundle is versioned.

Application description don't contains environment dependant information like domain name of you
server, database host etc.
It resemble similarity to Ports and Adapters concept from Hexagonal Architecture.
The same application bundle can be deployed to any environment that mean their requirements.
For example you deployed your my-new-app.1.0.1 to dev environment, and tested it.
Now you can deploy the same bundle to your stage or prod environments. No need to rebuild bundle for
specific environment.

## Application Description

appspec.yaml is main description file.
First it specs a name and a version of the application.

```yaml
name: myapp
version: 1.3.52
```

Then specify variables that need to be set depends on a environment. This variables can have default
value.

```yaml
environment:
  external:
    - DB_CONNECTION_STRING
    - REDIS_CONNECTION_STRING
    - SEND_GRID_API_HOST
    - ANDROID_APP_URL="https://play.google.com/store/apps/details?id=..."
    - IOS_APP_URL="https://apps.apple.com/..."
```

In `relative` section special type of variables are set. They used to set urls relatively to
application host.  
This free you from hassle of defining there variables independently for each environment.
But they still can be override by specific environment if needed.

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

`internal` sets private variables that can be used to setup interconnection between services or
common configuration.

```yaml
environment:
  internal:
    - LOGGING__LOGLEVEL__DEFAULT=Error
```

## Services

You can define two way services can be defined, app_services and extra_services.
The difference that app_services images always have same version as version defined in appspec.yaml.
For extra_services you use regular image version.
Lets check next definition:

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

During deployment next images will be pulled

- mycompany/web-app:1.0.1
- mycompany/backend-svc:1.0.1
- mycompany/admin-panel:1.0.1
- mycompany/admin-svc:1.0.1
- 3rd-party/auth-gateway:26.4

if you will try to explicitly specify version for app_services you will get validation error.

### Service types
There three service types:
 * public - exposed to external world, accessible by external url. Like `https:\\app.mycompany.com`
 * internal - only accessible to other app services
 * job - runs ones each deployment, used to setup things. not accessible to other services.

### Service env variables

By default no env variables passed to service.

You can pass specif variables from root `environment:`

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - DB_CONNECTION_STRING
      - REDIS_CONNECTION_STRING
      - SEND_GRID_API_HOST
```    

You can pass all variables from root `environment:`

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - $all
```    

You can pass all variables and override some of them.

```yaml
  backend-svc:
    image: mycompany/backend-svc
    environment:
      - $all
      - LOGGING__LOGLEVEL__DEFAULT=Warning
```    

## Configuration files

if you need environment dependant configuration files you can define them in next way:

```yaml
configs:
  # data is a name of config
  data:
    - countries_payment_settings.json    
```

Usage in service

```yaml
  backend-svc:
    image: mycompany/backend-svc

    # ...

    # will put countries_payment_settings.json at /data/countries_payment_settings.json path
    configs:
      - data: /data
```

## Secrets

All secrets avaliable for application is defined on root element `secrets:`.

```yaml
secrets:
  redis_password:
  db_password:
  admin_password:
  sendgrid_apikey:
```

By default secrets mount as files into "/etc/secrets/" directory.

Usage in service

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

Environment is defined in envspec.yaml
Consists of two parts ingress, and deployments.
Ingress defines root ingress ingress, and deployments defines applicators that deployed in this
environment.

### Ingress

Ingress defines hosts domain names

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

Lets define deployment config for myapp web app.

```yaml
deployments:
  myapp_prod:
    application:
      # application name, in this case `myapp` 
      name: myapp
      # extra: allow to define additional services that specific for given environment.   
      # for example on dev we can host database inside your cluster. But on prod we can use cloud managed database. 
      extra:
        - clinic.ext.yaml

    # spec file with environment variables defined.
    environment: clinic.env        
    # adds all files from ./data to config named data 
    configs:
      data: ./data

    # defines secrets avaliable for given application.
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

You can deploy multiple application in same environment. 
Lets deploy website, that contains frontend and healers cms services

```yaml
  website_prod:
    application:
      name: website
      # sets restriction on app version supported. In this case you can deploy any app version from 1.1.5 to 2.0.0
      version: ^1.1.5
    secrets:
      redis_password:
      db_password:
    # define default rehouse configuration for all services. Can be overridden for individual services. 
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

Each public service should have `host` and `prefix` defined

* `host` is abstract host name, if will be subtitled by real domain name from deploying environment.
* `prefix` is path prefix.

```yaml
services:
  myapp:
    host: myapp
    prefix: /
  admin-panel:
    host: myapp
    prefix: /admin-panel
```

In given example `admin-panel` will be serve on dev environment at
`https:\\myapp-dev.com\admin-panel`

# Prepare deployment artifacts
Build all docker images for given application

`cd web-app && docker build -f ./Dockerfile -t mycompany/web-app:latest .`

`cd backend-svc && docker build -f ./Dockerfile -t mycompany/backend-svc:latest .`

...

run `simpled verify`

It will check if all services from appspec.yaml have docker image built for them.

`simpled app-bundle create --registry mycompany=my-docker-registry.com --push-images`

Will tag all images with proper version from  appspec.yaml, and path them to your docker registry.
Then will create appname.$version.tar.gz artifact. Eg. myapp.1.0.52.tag.gz

upload it to github releases, or on s3 storage. if you use simpled compatible artifact storage, you can add `-- upload` parameter

`simpled app-bundle create --upload https:\\storage-domain.com\simpled`

## Create or update secrets

you can upload files from folder as secrets

`simpled secrets set myapp_prod ./myapp_prod_sercrets`

you can set secrets through command line

`simpled secrets set myapp_prod -f redis_password="${{ secrets.REDIS_PASSWORD }}" -f db_password="${{ secrets.DB_PASSWORD }}"`

Then apply generated manifests:

`kubectl apply -f k8s/`

# Deploy application

Navigate into the folder with envspec.yaml. eg deployments\prod

download app bundle into some folder. e.g. deployments\myapp.1.0.52.tag.gz

then run:

`simpled prepare-deployment myapp_prod --bundle deployments\myapp.1.0.52.tag.gz`

--bundle can point to a folder with appspec.yaml or tag.gz archive of that folder

and apply generated manifests:

`kubectl apply -f k8s/`

if you use simpled compatible artifact storage, no need to manually download:

`set SIMPLED_REPO_URL=https:\\storage-domain.com\simpled`
`set SIMPLED_API_KEY=my-api-key`

`simpled prepare-deployment myapp_prod --version 1.0.52`

and apply generated manifests:

`kubectl apply -f k8s/`

if all application requirements are meat in a given environment, the app will be deployed.
