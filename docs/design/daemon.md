# Design: simpled daemon

Status: draft · Targets: Docker (standalone and Swarm) and local. Kubernetes is out of scope for now.
The Docker gateway is Traefik only; the nginx gateway has been removed.

`simpled daemon` is a small control-plane service, built from the same binary as the
CLI, that runs next to the gateway of an environment. Phase 1 reports state. A later
phase also changes state. It replaces the per-deployment nginx sidecar
(`simpled-version-<deployment>`) that answers `/.well-known/simpled/version` today.

This is not part of the embedded docs (`src/docs.rs` lists its topics explicitly).

---

## 1. Goals and non-goals

Goals:

- **Public status.** Every gateway host keeps answering `version` and also gets a
  `health` answer. Neither reveals service names or topology.
- **Private status.** Per service, it reports replicas, task state, health, image,
  restarts, the last error and logs, behind a bearer token.
- **Two tokens.** A `read` token reads state and logs. A `manage` token can do
  everything `read` can, plus the write actions added later.
- **The private API goes through the gateway**, on a host that the env spec routes to
  it the same way it routes to any service.
- **The same API locally.** `simpled local run` serves it in-process.
- **Fix the multi-node gaps of the Swarm target** that need something running there
  (§7), above all Let's Encrypt with `gateway.global: true`. The gaps that need no
  daemon are fixed by the Swarm storage changes in simpled 2.0.0 (§8).

Non-goals for now: Kubernetes, multi-environment fleet views, a web UI, metrics
storage, and alerting. The daemon reports the current state. History lives elsewhere.

---

## 2. Where it runs: one daemon per environment, in the ingress stack

In the first sketch the daemon was per deployment, like the nginx sidecar. **That
does not hold once the daemon manages certificates.** The gateway belongs to the
environment, since it serves every deployment of the env spec, so the component that
issues its certificates must also belong to the environment. Two per-deployment
daemons would each run an ACME client for the same domains.

So:

| Target | How it runs |
|---|---|
| Swarm | Service `daemon` in the `ingress` stack, next to `traefik`. `replicas: 1`, constraint `node.role == manager`. |
| Standalone Docker | `docker run -d --name simpled-daemon` in `deploy.sh`, right before the gateway, the same way the gateway is started today. |
| Local | No container. The local gateway (`local_ingress.rs`) serves the routes in-process. |

The ingress stack is already rewritten by every deployment's `deploy.sh` from the
full env spec, so the daemon gets the same lifecycle and the same caveat as the
gateway: the last deploy writes its configuration.

**The version document now comes from labels.** An env-level daemon cannot hold a
static document for each deployment, because deploying one deployment would
overwrite the others. That is the reason the sidecar exists today. Instead, every
service in a deployment's stack gets labels when it is prepared:

```
simpled.deployment=prod
simpled.application=shop
simpled.version=1.2.3
simpled.deployed_at=1790945826
simpled.service=api          # the name from the spec, not the swarm name
```

The daemon reads them through the Docker API. This is more accurate than today: the
endpoint reports what is actually running, and it can tell when two versions are
running at once during a rollout. The JSON shape of `/version` stays the same.

The `simpled-version-<deployment>` service and `version_endpoint::service()` are
removed. `version_endpoint::routes()` stays: it is still the rule for which
deployment a domain answers for, and `health` uses it too.

### Image

The daemon ships as `ghcr.io/vkubiv/simpled:<version>`: the static musl build of the
same binary on `scratch`, with CA certificates for ACME. `release.yml` builds it for
amd64 and arm64. `prepare-deployment` pins the tag to its own version, so the
generator and the daemon always agree on labels and routes. `gateway.daemon.image`
overrides it for private registries.

How it is built (`Dockerfile`, `release.yml`):

- The `daemon-binary` jobs build `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl` natively, on the amd64 and arm64 runners, and check
  that the result is static.
- The `daemon-image` job puts both into one multi-platform image. The Dockerfile only
  copies files (the binary for `TARGETARCH`, and Alpine's CA bundle), so the build
  needs no emulation. The GitHub release is created only after the image is pushed.
- CI builds both binaries and images without pushing, so a change that breaks them
  fails a PR.
- The image runs as user 65532. The socket is `root:docker` (or `root:root` on Docker
  Desktop) with mode 660, so standalone `deploy.sh` adds the socket's group to the
  daemon's container. The Swarm stack runs it as root instead (see Docker access). Tested: as 65532 with `--read-only --cap-drop ALL` and that
  group added, the daemon reads the socket. It stops on SIGTERM in about half a
  second.
- **The ghcr package is private when it is first pushed.** It has to be made public
  once in the package settings, or deploy targets cannot pull it.

### Docker access

The daemon talks to `/var/run/docker.sock` through `bollard`, which also handles the
Windows named pipe for local runs. **This is the one real change in security
posture.** Until now, no generated container had the socket, and the socket is
effectively root on the host. Mitigations:

- The daemon is reachable only from the gateway, on its own network (§5), never from
  app containers.
- The public routes take no input beyond the Host header. Every other route needs a
  token, and with no tokens configured, those routes do not exist.
- The daemon only acts on objects it owns: anything labelled `simpled.*`, the
  `ingress` stack, and `simpled_*` configs and secrets. It refuses everything else
  even with the manage token.
- `read_only: true` root filesystem and no capabilities. On standalone Docker, a
  non-root user in the group that owns the socket, looked up by `deploy.sh` on the
  host that runs both. On Swarm, root: the daemon runs on any manager, and their
  `docker` groups need not have the same number. With the socket, root in the
  container gives nothing the socket does not already give.

The daemon needs write access to Docker from the ACME phase on (§7.1: it creates
certificate secrets and updates the gateway service), so a GET-only socket proxy
would only be a stopgap and is not planned.

---

## 3. Env spec

```yaml
gateway:
  hosts:
    web: shop.example.com
    ops: ops.example.com
  daemon:                       # optional block; the daemon runs regardless
    hosts:                      # where the private API is routed, like a service
      ops: { prefix: / }        # or e.g.  web: { prefix: /_simpled }
    read_token:                 # a secret source, same shapes as deployment secrets
      aws: prod/simpled/tokens
      jq: .read
    manage_token:
      env: SIMPLED_MANAGE_TOKEN
    image: registry.example.com/simpled:1.14.0   # optional override
```

- `daemon.hosts` uses the same shape and validation as a service's `hosts`
  (`prefix`, `prefixes`, `strip` and `body_limit`, with `strip` defaulting to true). A
  path that collides with an app route on the same domain fails validation with the
  usual error.
- **Without `daemon.hosts`, the private API is not routed**, even when tokens are
  set. Without `read_token`, the read routes do not exist. Without `manage_token`,
  the manage routes do not exist. Every combination is valid. Setting a token without
  `hosts` gets a warning, because the token would be unusable.
- Tokens use every secret source a deployment secret accepts (`env`, `file`, `aws`,
  `json_env`, ...), resolved by the same code. `aws` is deferred to
  `fetch-secrets.sh` as usual. Validation requires at least 32 characters.
- The block sits on `gateway` and not on a deployment, because the daemon belongs to
  the environment (§2).

How the tokens reach the daemon:

| Target | Delivery |
|---|---|
| Swarm | Content-addressed Swarm secrets (§8.2). **Not** a bind mount: the daemon may run on another manager than the one that ran `deploy.sh`. |
| Standalone | A bind mount of a file under `secrets/`, the same as deployment secrets today. |
| Local | Read in-process. In `localenv.yaml` the tokens are optional while the gateway binds a loopback address. With `--bind 0.0.0.0` they are required, if `daemon.hosts` is set. |

---

## 4. HTTP API

### Public, on every gateway host

| Route | Answer |
|---|---|
| `GET /.well-known/simpled/version` | Unchanged: `{"application","version","deployment","deployed_at"}` for the deployment that owns the host (`version_endpoint::routes`). |
| `GET /.well-known/simpled/health` | `{"status":"healthy\|degraded\|down","converged":bool,"version":"1.2.3"}`, with status 200 when `healthy` and 503 otherwise, so uptime checks need no JSON parsing. |

Health is computed for the deployment that owns the host:

- `down`: some service has zero running tasks while it should have at least one.
- `degraded`: some service runs fewer tasks than desired, or has a task that is
  `unhealthy`.
- `healthy`: neither of the above.
- `converged`: every running task of the deployment carries this deployment's
  `simpled.version` label, and no update is in progress (`UpdateStatus.State` is not
  `updating` or `rollback_started`).

`converged` closes the gap the reference states today: *"the answer says the
deployment's stack was updated, not that every service in it is healthy."*

### Private, under `daemon.hosts`, with `Authorization: Bearer <token>`

| Route | Token | Returns |
|---|---|---|
| `GET /api/v1/status` | read | Every deployment: version, health, `converged`, services with desired/running counts. One call for a dashboard or `simpled status`. |
| `GET /api/v1/deployments/{d}/services` | read | Per service: image, desired/running, health, restarts in the last hour, last task error, update state. |
| `GET /api/v1/deployments/{d}/services/{s}/tasks` | read | Per task: node, state, since, exit code, error (Swarm). Per container locally and on standalone. |
| `GET /api/v1/deployments/{d}/services/{s}/logs?tail=200&since=10m&follow=1&timestamps=1` | read | `text/plain` stream. On Swarm this is the service log aggregated across nodes, read from the manager. |
| `GET /api/v1/gateway` | read | Gateway tasks per node, and certificates with their domains and expiry dates (§7.1). |
| `GET /api/v1/nodes` | read | Swarm nodes: role, availability, state, labels. |
| `POST /api/v1/deployments/{d}/services/{s}/restart` | manage | *Phase 4.* `docker service update --force`. |
| `POST /api/v1/deployments/{d}/services/{s}/scale` | manage | *Phase 4.* |
| `POST /api/v1/deployments/{d}/rollback` | manage | *Phase 4.* Uses the previous spec Swarm keeps. |
| `POST /api/v1/gateway/certificates/renew` | manage | *Phase 3.* Forces a renewal. |

Rules:

- `{d}` and `{s}` are the names from the env spec (`prod`, `api`), not Swarm names.
  A name the daemon does not own is a 404, not a 403, so names cannot be probed.
- A wrong or missing token gets 401 and a token without enough rights gets 403.
  Tokens are compared in constant time. Failed attempts are rate-limited per client
  IP, using the real client IP that the gateway already forwards (`trusted_proxies`).
- Logs are as sensitive as the secrets that leak into them. The reference must say
  that the read token is a credential, not a "viewer" password.
- Rotation means redeploying with a new value. One token of each kind is enough for
  now.

### CLI client

`simpled status` and `simpled logs <deployment> <service> [-f]` find the URL from
the env spec (`daemon.hosts`) and the token from `SIMPLED_TOKEN`. For CI there is
`simpled wait --version 1.2.3 --timeout 10m`, which polls `/health` on the
deployment's primary host until it reports `converged` and `healthy` at that version.
Today the docs suggest polling `/version` for this, which only proves the stack was
updated.

---

## 5. Networking

```
            :80/:443 (host mode, every node with global: true)
 client ──► gateway ──── common_network ────► app services
              │
              └──── simpled_control (overlay, attachable: false) ──► daemon :8080 (API + public)
                                                                     daemon :8081 (ACME challenges)
```

- `simpled_control` contains only the gateway and the daemon. App containers sit on
  `common_network` and cannot reach the daemon except through gateway routes.
- The gateway routes `/.well-known/simpled/{version,health}` on each owned host, the
  `daemon.hosts` paths, and, from Phase 3, `/.well-known/acme-challenge/` on every
  domain to the daemon.
- Standalone Docker uses a bridge network named `simpled_control`, with the same
  layout.

---

## 6. Locally

`simpled local run` and `simpled test e2e` already run the gateway in-process
(`local_ingress.rs`, axum). The daemon's router becomes a library module
(`src/daemon/`) that both the `daemon` subcommand and the local gateway mount:

- `version` and `health` are added on every local port (`version` is already there).
  Health reads the compose project's containers through the local Docker API, so the
  local stack is filtered by `com.docker.compose.project`, not by Swarm labels.
- The private API is mounted under `daemon.hosts` when `localenv.yaml` sets it, with
  the auth relaxation described in §3.
- No ACME locally.

`simpled test e2e` can then wait on `/health` instead of probing each service.

---

## 7. Problems it can solve

Each problem is followed by whether the daemon is actually the right fix. Some are
not, and those should be fixed without it.

### 7.1 Let's Encrypt with more than one gateway node *(daemon: yes)*

**Before the daemon** (simpled 2.0.x). The Swarm Traefik gateway was its own ACME
client and stored `/letsencrypt/acme.json` in a bind-mounted directory of the node it
ran on. Hence:

- `global: true` and `tls.letsencrypt` are rejected together: each task would run its
  own ACME client, and an HTTP-01 challenge lands on whichever node the balancer
  picks, usually not the one that asked.
- With a single replica, `acme.json` lives on one node. When Swarm reschedules the
  gateway to another node, it starts with an empty store and issues every certificate
  again. Let's Encrypt allows 5 duplicate certificates per week, so a few reschedules
  in a bad week lock the domain out.

**With the daemon.** The daemon is the only ACME client, using the `instant-acme`
crate. Traefik no longer runs ACME at all: it only reads PEM files.

As built (steps 5a and 5b), it differs from the plan below in three places:

- **One certificate, and one secret, per domain.** A domain whose DNS is not ready
  fails alone instead of blocking a certificate for every name. The PEM holds the
  chain and the key, and Traefik uses it as both `certFile` and `keyFile`, at
  `/run/secrets/simpled_tls/<domain>.pem`.
- **A self-signed placeholder instead of Traefik's default certificate.**
  `step tls-secrets` creates one for a domain with no certificate, so the ingress
  stack always mounts one secret per domain, and a redeploy looks up which.
- **The HTTPS redirect has priority 1.** Traefik's entry-point redirect normally
  outranks every router; at 1, the challenge router on `web` is reached first.

Step 5d lifted the `global` + `letsencrypt` rejection, and `/health` reports `tls`
(`valid`, `expiring` within 7 days, or `self-signed`), which degrades a healthy
deployment. The placeholder counts too: browsers warn while it is served.

Tested against Pebble, Let's Encrypt's test server: two domains issued through real
HTTP-01 validation through Traefik, one rolling restart of the gateway, and a
redeploy that kept both certificates and restarted nothing.

1. **Challenges.** Every gateway task, on every node, proxies
   `/.well-known/acme-challenge/*` to the daemon over `simpled_control`. It no longer
   matters which node the challenge lands on. This is what lifts the
   `global` + `letsencrypt` restriction.
2. **Certificates are Swarm secrets.** On issue or renewal, the daemon creates
   `simpled_tls_<hash>` holding the certificate chain and the key, labelled
   `simpled.tls.domains=…` and `simpled.tls.not_after=…`. Traefik loads it through
   the file provider:
   ```yaml
   tls:
     certificates:
       - certFile: /run/secrets/simpled_tls_crt
         keyFile:  /run/secrets/simpled_tls_key
   ```
   Secrets reach every node through Raft (the store the Swarm managers replicate
   among themselves), so there is no shared storage and nothing to encrypt by hand.
3. **Delivery.** The daemon runs a rolling `service update` of the gateway onto the
   new secret, one task at a time, using the gateway's own `update` policy.
   `deploy.sh` looks up the current certificate secret by label and passes it into
   the ingress stack, so a redeploy keeps it. On the first deploy there is no secret
   yet, so the gateway starts with Traefik's default certificate until the first
   certificate is issued.
4. **No state to keep.** The daemon never reads a certificate back: the domains and
   the expiry date are in the labels, which the API returns. The ACME account is not
   kept either. A new account on each daemon start is fine, because the rate limits
   that matter are on certificates, not on accounts.
5. **Renewal** happens 30 days before expiry, with a jittered retry. Expiry shows up
   in `GET /api/v1/gateway`, and `/health` reports `degraded` when a certificate is
   within 7 days of expiry.
6. **Migration.** `simpled tls import acme.json` turns the certificates and keys in an
   existing Traefik `acme.json`, where both are stored base64-encoded, into
   `simpled_tls_*` secrets. Upgrading then issues nothing new.
7. **Standalone Docker** has a single host, so the daemon writes the PEM files into a
   directory that Traefik's file provider watches, and Traefik picks them up without
   a restart.

**What it costs.** Each renewal causes a rolling restart of the gateway, about every
60 days. With host-mode ports and `stop-first`, each node drops its connections for
the moment its task restarts. A load balancer that checks `/ping` stops sending to
the node first. With DNS round-robin straight to the nodes, a few requests can fail
once every two months.

**What it does not depend on.** Gateways never depend on the daemon at runtime. If
the daemon is down, renewals wait, with 30 days of margin.

### 7.2 Bind-mounted files on a multi-node Swarm *(daemon: no)*

The generated stacks bind-mount files that exist only in the deploy directory of the
node that ran `deploy.sh`: the gateway's `traefik.yml` and `dynamic_conf.yml`,
each deployment's `configs/` and `run/secrets/`, every named volume
(`./volumes/<name>`) and every relative bind path. A task scheduled on any other
node, which is every node with `global: true`, finds no bind source. Swarm rejects
it, or, worse, the task starts against a directory that `mkdir -p` created empty on
an earlier run.

This does not need the daemon. It is fixed by the Swarm storage changes in §8.

### 7.3 "Is the deploy actually done?" for CI *(daemon: yes)*

`deploy.sh` waits with `--detach=false`, but only the pipeline that ran it knows the
result. Anything else, such as a second pipeline, a smoke test or a status page, can
only poll `/version`. `/health` with `converged`, together with `simpled wait`, gives
them a real signal that also survives regressions after the deploy, such as a crash
loop an hour later.

### 7.4 Logs and state without SSH *(daemon: yes)*

Today, finding out why `api` is restarting means SSH to a manager, then
`docker service ps --no-trunc` and `docker service logs`. The read token and
`simpled logs` replace that, and the access can be given to people who should not
have SSH.

### 7.5 Later, not now

These all need the daemon on every node (`mode: global`, an "agent" role), because
the Docker API of a manager does not reach containers on other nodes:

- Per-container CPU and memory stats from workers.
- `docker image prune`, which today runs only on the node that ran `deploy.sh`, so
  worker disks fill up.
- Free disk space per node, and per `node`-bound volume (§8.3).

And with the manage token: restart, scale, rollback, re-running a failed job, and
refreshing deferred `aws` secrets without a full redeploy.

---

## 8. Swarm storage (simpled 2.0.0)

These are breaking changes, so they ship together as simpled 2.0.0. They do not
depend on the daemon, and the daemon's certificates (§7.1) and tokens (§3) build on
them.

The principle: **every file a task needs either travels through Raft (configs,
secrets), or belongs to a volume whose location the env spec declares.** Nothing
relies on being on the node that ran `deploy.sh` without saying so.

### 8.1 `multi_node`

```yaml
type: docker
swarm_mode: true
multi_node: true        # the default
```

- **`true`, the default:** the environment is treated as multi-node even when the
  cluster has one node. Every rule in §8.3 and §8.4 applies, so a spec that passes
  validation keeps working when nodes are added. You find out on a one-node cluster,
  not after adding the second.
- **`false`:** the environment lives on one machine. Unbound volumes and relative
  bind paths are allowed, as in 1.x. Two safeguards keep that true:
  1. **`deploy.sh` stops if the swarm has more than one schedulable node**, before it
     creates or deploys anything. It counts the nodes that are `Ready` and `Active`
     (`docker node ls --format '{{.Status}} {{.Availability}}'`). `deploy.sh` already
     runs on a manager, so it can list nodes. A drained node does not count, because
     it gets no tasks: a new node can join in `drain`, be prepared, and be activated
     once the spec says `multi_node: true`. The message names the nodes and both ways
     out:
     ```
     This swarm has 2 nodes that accept tasks (manager-1, worker-1), but the env spec
     sets multi_node: false. Set multi_node: true and bind the volumes (simpled docs
     reference --section volumes), or drain the extra nodes
     (docker node update --availability drain worker-1).
     ```
  2. **Every service, job and the gateway is pinned to that one schedulable node.**
     `deploy.sh` takes its ID from the same `docker node ls` output and exports it
     for a `node.id == ${SIMPLED_NODE_ID}` constraint, which `docker stack deploy`
     interpolates. That node is not necessarily the one running `deploy.sh`: with a
     drained manager and one active worker, everything runs on the worker. The check
     alone runs only at deploy time, while Swarm can reschedule a task onto a newly
     added node at any moment in between, after a crash or a reboot. With one node
     the pin costs nothing. With no schedulable node at all, `deploy.sh` stops and
     says so.
- `multi_node` is Swarm only, and is rejected anywhere else, like `gateway.global`.
- `multi_node: false` together with `gateway.global: true` is an error: a gateway on
  every node, but everything pinned to one node.

### 8.2 Configs and secrets: content-addressed Swarm objects

Every config file, file secret and gateway file (`traefik.yml`, `dynamic_conf.yml`,
certificates) becomes a native Swarm config or secret, whatever `multi_node` says, so
there is one mechanism in the generator.

1. **Names carry a hash of the content:** `prod_db_password_3f9a1c2e4b5d`, the base
   cut to fit Swarm's 64-character limit. The hash is taken on the deploy target,
   from the file actually uploaded, not when the deployment is prepared: a secret
   with an `aws` source only has a value there, and this treats every object the
   same way. The name is exported in a variable that the stack files interpolate
   (`name: ${SIMPLED_OBJECT_<n>}`). The same value gives the same name, so nothing
   happens. A new value gives a new name, so it is a new object. Nothing is ever
   modified, which is what makes Swarm's immutability painless.
2. **`./simpled step sync-objects` creates them** (§8.5). It reads
   the deployment's `plan.json` and is idempotent: an object
   that already exists under its hashed name is left alone. Each object is labelled
   with its owner (`simpled.deployment=prod`, §8.6) and its base name
   (`simpled.object=prod_shop-db_password`). The stack files reference them as
   `external: true`. They are created by this step and not by `docker stack deploy`, because jobs run through
   `docker service create` and need the objects to exist (`--secret source=…,target=…`),
   and `aws` secrets exist as files only after `fetch-secrets.sh` has run on the target.
3. **Swarm restarts only what changed.** A changed secret changes the service spec,
   so Swarm does a rolling update of exactly the services that use it, following
   their `update` policy.
4. **Pruning.** After a successful deploy, every labelled object of the deployment
   that is not current is removed, except the previous generation, so that
   `docker service rollback` still works. `docker secret rm` refuses an object that a
   service still uses, so the prune cannot break anything running.
5. **Limits enforced by validation:**
   - At most 500 KB per object. A larger config file fails with a clear message. It
     never falls back to a bind mount without saying so.
   - Configs and secrets both accept any absolute target path, so a secret keeps the
     path the app spec gives it (`/etc/app/key.pem`). Checked on Docker 29.7 with
     `docker service create` and with `docker stack deploy` and `external: true`
     objects: the file appears read-only on tmpfs at that path. A relative target
     stays under `/run/secrets/`. (When testing from Git Bash on Windows, set
     `MSYS_NO_PATHCONV=1`, or the shell rewrites `/etc/...` into a Windows path
     before Docker sees it.)
   - Mode `0444`, so a service that does not run as root can still read its secret.
6. Env-variable secrets (`variable:`) are unchanged. They live in the service spec,
   which already reaches every node.

Standalone Docker keeps bind mounts, which are fine on a single host.

### 8.3 Volumes: bound per volume in the env spec

The app spec only names its volumes. In ports-and-adapters terms, the env spec is
the adapter: it says where each volume lives in this environment.

```yaml
deployments:
  prod:
    volumes:
      pg-data:
        node: db-1                        # a Docker volume on db-1
      uploads:
        nfs: fs-12ab.efs.eu-west-1.amazonaws.com:/shop/uploads
      media:
        path: /mnt/shared/shop/media      # a host path that exists on every node
      legacy-data:
        node: app-1
        path: /srv/shop/legacy            # a host directory on one node
```

| Binding | Generated | Where its services can run |
|---|---|---|
| `node` | A Docker named volume (local driver). Docker creates it on the node where the task lands, so a first deploy finds nothing missing. | Pinned with `node.hostname == <node>` |
| `node` + `path` | A bind mount of that directory | Pinned to that node |
| `path` | A bind mount of a path that must exist on every node, such as a shared mount | Anywhere |
| `nfs` | A Docker volume with `type=nfs` driver options (`nfs: { address, path, options }` in the long form). Each node mounts it itself when a task needs it, so nodes need no `/etc/fstab` entry. | Anywhere |

Exactly one of `node`, `path` (with or without `node`) or `nfs`.

**A service follows its volumes**, so stateless is not a setting: a service without
volumes runs anywhere. Validation, with `multi_node: true`:

1. **An unbound volume is an error.** The error lists every unbound volume with a
   ready-to-paste `volumes:` block.
2. **A service mounting a pinned volume gets that node's constraint**, merged with its
   own `placement`. Jobs get it too.
3. **Two volumes pinned to different nodes on one service is an error**, since no
   node can satisfy both.
4. **A pinned volume with `update.order: start-first` is an error.** For a moment two
   tasks would write the same directory, which is how a database gets corrupted.
   Swarm's default `stop-first` is required.
5. **`replicas > 1` on a pinned volume is a warning.** Every replica lands on the same
   node and shares the directory.
6. **A database image on `nfs` or a shared `path` is a warning:** postgres, mysql,
   mariadb or mongo, matched by image name. NFS locking and fsync semantics are a
   known source of corrupted databases.
7. A binding for a volume the app spec does not declare is an error, as with any
   unknown name.

Bindings are Swarm only for now. Standalone Docker could take `path` and `nfs`, but
its `docker run` commands mount no named volumes at all today, which has to be fixed
first. Local runs and Kubernetes are unchanged.

**Migrating from 1.x:** data lives in `<deploy dir>/<deployment>/volumes/<name>` on the
node that ran `deploy.sh`. Binding the volume with `node` plus a `path` pointing at
that directory keeps the data where it is.

### 8.4 Relative bind paths

`./data:/x`, whether in the app spec or in a deployment's service `volumes`, is a
directory on whichever node ran `deploy.sh`. With `multi_node: true` it is an error on
Swarm, and the message points to the alternatives: a config for files the service
only reads, or a named volume bound in the env spec for anything it writes.
Absolute host paths (`/var/run/docker.sock:/var/run/docker.sock`) stay allowed,
because they name something that exists on every node by intent.

### 8.5 Deploy steps: `deploy.sh` calls the simpled binary

`deploy.sh` stays the readable outline of a deploy: its order, its phases and every
`docker stack deploy` are visible. The steps that would otherwise be a lot of generated
shell become calls to a simpled binary shipped next to it.

```sh
#!/bin/sh
set -e
cd "$(dirname "$0")"

. ./fetch-secrets.sh
./simpled step lock shop
./simpled step check-nodes                 # only with multi_node: false
./simpled step sync-objects                # environment objects (ingress/plan.json)
./simpled step sync-objects shop           # this deployment's objects (shop/plan.json)

docker stack deploy -c ingress/docker-compose.yaml ingress --prune --detach=false

echo '== Phase 1/3: services the jobs depend on =='
docker stack deploy -c shop/docker-compose.deps.yaml shop --with-registry-auth --detach=false
echo '== Phase 2/3: jobs =='
./simpled step run-job shop migrate
echo '== Phase 3/3: the rest of the stack =='
docker stack deploy -c shop/docker-compose.yaml shop --with-registry-auth --prune --detach=false

./simpled step prune-objects shop          # keeps the previous generation for rollback
./simpled step unlock shop
```

**What moves into steps:** `run_job` (today it builds a `docker service create` of
about 30 lines per job and polls the result), `wait_healthy` on standalone Docker,
creating and pruning objects (§8.2), the node check (§8.1), the node-ID pin, and later
the lookup of the certificate secret (§7.1). `docker stack deploy` and
`fetch-secrets.sh` stay shell.

**The binary.** `prepare-deployment` puts `docker-deploy/simpled` next to
`deploy.sh`, so the same `rsync` carries it and the version on the server is always
the one that prepared the deployment:

- `--target-binary <path>` (or `SIMPLED_TARGET_BINARY`) names the binary to ship,
  for an unreleased build or an offline machine.
- Otherwise, when `prepare-deployment` runs on Linux on the same CPU type as the
  target, it copies itself.
- Otherwise it downloads that same version's release asset for the target and
  verifies it against the release's checksums, with the code `simpled update` uses.
- The target is `linux-amd64` unless the env spec sets the top-level
  `target_platform: linux-arm64` (Swarm only). It belongs in the env spec because
  the platform is a property of the environment.
- Releases from 2.0.1 on build `simpled_linux_arm64` (`aarch64-unknown-linux-gnu`,
  natively on GitHub's arm64 runner). **Open:** both Linux binaries link glibc
  dynamically, so a target needs a glibc no older than the runner's; static (musl)
  builds would remove that.
- Before acting, every step checks that its plan was written by the same simpled
  version, so files from two prepares mixed on a target fail loudly.

`step` commands are hidden from the main `--help`. Each one prints what it does
(`created prod_db_password_3f9a1c2e`, `job migrate: completed in 14s`), so the deploy
log reads as before.

**Plans.** Steps read only a plan written by `prepare-deployment`, never the env or app
spec. Those cannot be resolved again on the server: they depend on CI environment
variables, on files next to the spec, on `extends`, and on the downloaded bundle.

| File | Holds |
|---|---|
| `ingress/plan.json` | The environment: gateway, daemon, environment objects, `multi_node`, the env spec hash, the simpled version |
| `<deployment>/plan.json` | One deployment: its objects with their hashes, its jobs, constraints and phases |

A plan holds no more than the stack file does. File secrets appear only as paths, and
`aws` secrets only as variables that `fetch-secrets.sh` exports. A literal secret
mounted as an environment variable appears in a job's arguments with its value, as it
does in the stack file's `environment:` and in the service spec Docker keeps. It is
its own struct with its own format, not a serialization of
`EnvironmentResolvedSpec`, so refactoring the resolver does not change the file.

### 8.6 Several deployments in one env spec

Every deployment of an env spec is deployed on its own, into its own stack, and every
`deploy.sh` also redeploys the ingress stack from the full env spec. What keeps them
from interfering with each other:

1. **One directory per deployment on the target**, for example
   `rsync -a docker-deploy/ host:~/deploy/shop/`. The reference recommends this layout.
   Nothing generated is shared between two deployments: each has its own
   `deploy.sh`, binary, plans and `ingress/` copy.
2. **Owners on every object, and pruning that stays within its owner.** Deployment
   objects are labelled `simpled.deployment=<name>`, and `prune-objects shop` touches
   only those. Environment objects (gateway files, daemon tokens, certificates) are
   labelled `simpled.scope=environment`, and only the ingress side prunes them.
3. **No downgrades of the environment.** The gateway service is labelled with the
   simpled version and the deployment that deployed it. `step check-gateway`, right
   before the ingress stack deploys, stops a deploy that would replace it with a
   lower simpled version, unless `SIMPLED_ALLOW_DOWNGRADE=1` is set. This guards the
   daemon, whose image is pinned to the simpled version. Instead of an env spec
   hash, the step compares the gateway's routing config that runs with the one about
   to replace it: both are content-addressed Swarm configs, and configs can be read
   back, so it prints the changed lines. A deploy from a stale checkout shows in the
   log as routes it removes.
4. **One deploy at a time per swarm.** `step lock` creates the config
   `simpled_deploy_lock`, labelled with the deployment, the host and the time.
   Creating a config whose name already exists fails, and Raft keeps that consistent
   across the cluster, so it works as a cluster-wide lock. A second deploy waits up to
   10 minutes (`SIMPLED_LOCK_WAIT`), then fails, naming who holds the lock. When that
   lock is older than 30 minutes, the error says the deploy holding it probably
   died, and that `step unlock --force` clears it; nothing is broken automatically.
   The lock carries a random token, so `step unlock` only releases this deploy's
   own lock. `deploy.sh` releases it on exit through a `trap`, also when a step
   fails.

Out of scope: removing a deployment from the env spec leaves its stack running, since
nothing deletes it. The gateway stops routing to it. A later
`step remove-deployment` can handle that.

### 8.7 What 2.0.0 breaks

| 1.x spec | 2.0.0 |
|---|---|
| Swarm, named volumes without a binding | Error with a ready-to-paste block. Or set `multi_node: false` to keep the 1.x behavior on one node. |
| Swarm, relative bind paths | Error. Or `multi_node: false`. |
| Swarm, config or secret files on a shared volume | Nothing to change: they become Swarm objects automatically. |
| Target on a different CPU type than CI (for example arm64) | Set `target_platform`. 2.0.0 also needs `--target-binary`; 2.0.1 releases the arm64 binary. |
| `gateway.type: nginx`, `tls.secret` on Docker | Rejected. Removed on main after 1.13.1, so first released in 2.0.0. |
| Several deployments sharing one directory on the target | Each needs its own directory (§8.6). |

The user-facing version of this table is "Upgrading to 2.0.0" in `docs/reference.md`,
and the release notes are `docs/releases/2.0.0.md`.

---

## 9. Phases

1. **Replace the sidecar.** Add the `simpled daemon` subcommand, the version route
   reading labels, the labels added in `prepare-deployment`, the image in
   `release.yml`, the daemon in the ingress stack (Swarm) and `deploy.sh`
   (standalone), and the `simpled_control` network. Remove
   `simpled-version-<deployment>`. Behavior does not change.
2. **simpled 2.0.0: Swarm storage and deploy steps (§8).** `multi_node`,
   content-addressed configs and secrets, volume bindings, the relative-path rule,
   `simpled step` with the shipped binary and the plans, the lock and the downgrade
   guard. This does not depend on the daemon and can ship before phase 1, or
   together with it.
3. **Status.** Public `/health`, `gateway.daemon` in the spec, tokens (as Swarm
   secrets, §8.2), the private read API, logs, the local mount, `simpled status`,
   `simpled logs` and `simpled wait`.
4. **ACME.** Challenges, certificates as secrets, the rolling gateway update, renewal,
   `simpled tls import`, and lifting the `global` + `letsencrypt` rejection.
5. **Manage.** Restart, scale and rollback with the manage token.
6. **Agent mode.** A global per-node daemon for stats, prune and disk space.

---

## 10. Open questions

1. **Always on, or opt-in?** This draft runs the daemon in every Docker environment,
   because it replaces the version sidecar and later issues certificates. That means
   every environment now has a container with the Docker socket. The alternative is
   `gateway.daemon.enabled: false`, which brings back a static version answer with
   no health.
2. ~~**Secret target paths.**~~ Resolved: absolute targets work for secrets too
   (§8.2).
4. **Health of excluded or zero-replica services.** These probably do not count
   toward health, but some services are scaled to 0 on purpose.
5. **Several deployments on one domain.** `/health` follows `/version`'s ownership
   rule. A shared domain that no deployment owns answers neither.
