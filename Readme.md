# Simple deployment - simpled

The idea is to bridge the gap between simple deployment configurations like docker-compose and super complicated Kubernetes (k8s) setups.

A lot of projects need something more flexible than docker-compose, but they can easily drown in Kubernetes configuration complexity.

Kubernetes doesn't give you a predefined structure for your deployment, so you need to invent one yourself. This is tricky and hard to do for the first time.
To make things flexible enough, but at the same time simple to use, `simpled` borrows approaches from programming like modularity, isolation, self-verification, consistency checking, and comprehensive human-readable errors.
It also borrows the app bundle concept from mobile development.

## Documentation

- [Tutorial](docs/tutorial.md) — step-by-step guide from a blank directory to a running Kubernetes deployment
- [Examples](docs/examples.md) — annotated real-world configurations covering common patterns
- [Reference](docs/reference.md) — every field in `appspec.yaml` and `envspec.yaml`, plus CLI flags and generated output
- [CI/CD Integration](docs/cicd.md) — automating builds and deployments with GitHub Actions
- [For AI coding agents](docs/agent.md) — condensed manual: the mental model, the rules that break builds, and an error-to-fix table

All of it is embedded in the binary, so you never have to leave the shell to look a field up:

```bash
simpled docs                                  # list the topics
simpled docs reference --outline              # section headings of one topic
simpled docs reference --section secrets      # just that section
simpled docs search "working_dir"             # search every topic
```

### Working with a coding agent

`simpled init-agent` writes a skill file into a project so Claude Code (and other agents
that read `.claude/skills/`) find these docs on their own:

```bash
cd my-project
simpled init-agent          # writes .claude/skills/simpled/SKILL.md
```

The agent then looks fields up with `simpled docs` instead of guessing at them. Run
`simpled docs agent` yourself to see what it reads, or `simpled init-agent --stdout` to
print the skill without writing it.

## Installation

Download the binary for your platform from the
[latest release](https://github.com/vkubiv/simpled/releases/latest) and put it on your
`PATH`. Once installed, `simpled update` fetches new releases and verifies their checksum
before installing them.

## Sixty-second tour

An application is described once, in `appspec.yaml`, and deployed to any environment
described in `envspec.yaml` (or `localenv.yaml` for a laptop):

```bash
# In the application repository: tag and push the images, write the bundle.
simpled app-bundle create --registry mycompany=registry.example.com --push-images

# In the environment repository: turn bundle + envspec into plain manifests.
simpled prepare-deployment myapp_prod --app-bundle myapp.1.0.52.tar.gz
kubectl apply -f manifests/        # Kubernetes
./docker-deploy/deploy.sh          # Docker or Swarm

# On a laptop: gateway, services and compose file, from the same appspec.
simpled local run

# Bring the local stack up, run the e2e suite from testspec.yaml, tear it down.
simpled test e2e
```

Every field, flag and generated file is described in the [Reference](docs/reference.md);
the [Tutorial](docs/tutorial.md) walks through the whole flow from an empty directory.

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
