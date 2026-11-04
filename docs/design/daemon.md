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
  (§7), above all Let's Encrypt with `gateway.global: true`.

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
- `read_only: true` root filesystem, no capabilities, and a non-root user in the
  group that owns the socket.

The daemon needs write access to Docker from Phase 3 on (§7.1: configs and the state
store), so a GET-only socket proxy would only be a stopgap and is not planned.

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
| Swarm | Native Swarm secrets in the ingress stack (`secrets: simpled_read_token: file: ...`), named with a content hash because Swarm secrets are immutable. **Not** a bind mount: the daemon may run on another manager than the one that ran `deploy.sh` (§7.2). |
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
                                                                     daemon :8081 (ACME challenges, cert feed)
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

**Today.** The Swarm Traefik gateway is its own ACME client and stores
`/letsencrypt/acme.json` in a bind-mounted directory of the node it runs on. Hence:

- `global: true` and `tls.letsencrypt` are rejected together: each task would run its
  own ACME client, and an HTTP-01 challenge lands on whichever node the balancer
  picks, usually not the one that asked.
- With a single replica, `acme.json` lives on one node. When Swarm reschedules the
  gateway to another node, it starts with an empty store and issues every certificate
  again. Let's Encrypt allows 5 duplicate certificates per week, so a few reschedules
  in a bad week lock the domain out.

**With the daemon.** The daemon replaces Traefik as the ACME client, using the
`instant-acme` crate:

1. **Challenges.** Every gateway task, on every node, proxies
   `/.well-known/acme-challenge/*` to the daemon over `simpled_control`. It no longer
   matters which node the challenge lands on. This is what lifts the
   `global` + `letsencrypt` restriction.
2. **Storage.** The ACME account key and the certificates must survive the daemon
   moving to another node, without shared storage:
   - *Swarm:* encrypted blobs in Swarm configs. Raft, the store the Swarm managers
     replicate among themselves, holds both the configs and the key, so the state
     survives the loss of any node. See §7.1.1 for the encryption.
   - *Standalone:* a directory, `./daemon-state/`, mounted into the container.
3. **Delivery to the gateways.** Traefik gets a second provider, `providers.http`,
   polling `http://daemon:8081/traefik/tls` every 10 seconds. It returns only
   `tls.certificates` with the PEM inline. Routes stay in the file provider, so
   routing never depends on the daemon. Certificates are picked up without a
   restart.
4. **Renewal** happens 30 days before expiry, with a jittered retry. Expiry shows up
   in `GET /api/v1/gateway`, and `/health` reports `degraded` when a certificate is
   within 7 days of expiry.
5. **Migration.** On its first start, the daemon imports an existing Traefik
   `acme.json` (account and certificates) if it finds one, so upgrading does not
   issue every certificate again.

**Failure mode to accept.** A gateway task that *restarts* while the daemon is down
serves Traefik's default certificate until the daemon is back. A running gateway
keeps the certificates it last fetched. On Swarm the daemon is rescheduled within
seconds, so this window is short and only affects restarted tasks.

#### 7.1.1 Encrypting the state on Swarm

- **The key** is 32 random bytes in the Swarm secret `simpled_state_key`. `deploy.sh`
  creates it once and never touches it again:
  `docker secret inspect simpled_state_key >/dev/null 2>&1 || head -c 32 /dev/urandom | docker secret create simpled_state_key -`.
  Only the daemon service references it, so Swarm delivers it only to that task, in
  memory, at `/run/secrets/simpled_state_key`.
- **Each blob** is a config named `simpled_state_<kind>_<hash>`, holding
  `version byte || 24-byte random nonce || XChaCha20-Poly1305 ciphertext`, using the
  `chacha20poly1305` crate. The config name is the associated data, which ties each
  ciphertext to its own name so blobs cannot be swapped. Configs are immutable, so a
  write creates a new config, switches over to it, and deletes the old one.
- **Why configs and not secrets:** the Docker API never returns a secret's contents,
  so reading one means mounting it into a task, which would restart the daemon on
  every renewal. Configs can be read back (`GET /configs/{id}`), but they are shown
  in plaintext by `docker config inspect`, support dumps and API scrapers. Hence the
  fixed key in a secret, and the changing data in configs encrypted under that key.
- **The threat model:** this protects private keys from anything that can read
  configs. It does not protect against root on a manager or access to the Docker
  socket, because they can start a task that mounts the secret. Nothing inside Swarm
  prevents that.
- **Losing the key:** the state cannot be read. The daemon logs it, starts with an
  empty store and issues the certificates again, which counts against the
  rate limit.
- **Rotating the key:** create `simpled_state_key_2` and mount both. The version byte
  names the key a blob was encrypted with, and the daemon re-encrypts on its next
  write.

### 7.2 Bind-mounted files on a multi-node Swarm *(daemon: no, use Swarm configs)*

The generated stacks bind-mount files that exist only in the deploy directory of the
node that ran `deploy.sh`: the gateway's `traefik.yml` and `dynamic_conf.yml`,
and each deployment's `configs/` and `run/secrets/`. A task
scheduled on any other node, which is every node with `global: true`, finds no bind
source. Swarm rejects it, or, worse, the task starts against a directory that
`mkdir -p` created empty on an earlier run.

This does not need the daemon. **The fix is native Swarm `configs:` and `secrets:`
in the stack files**, named with a content hash because they are immutable.
`docker stack deploy` uploads them to Raft and every node gets them. It is worth
doing before Phase 3, because the daemon's own tokens and the Traefik static config
need it too.

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
- Warning when a service with a bind-mounted named volume (node-local data) gets
  rescheduled to another node.

And with the manage token: restart, scale, rollback, re-running a failed job, and
refreshing deferred `aws` secrets without a full redeploy.

---

## 8. Phases

1. **Replace the sidecar.** Add the `simpled daemon` subcommand, the version route
   reading labels, the labels added in `prepare-deployment`, the image in
   `release.yml`, the daemon in the ingress stack (Swarm) and `deploy.sh`
   (standalone), and the `simpled_control` network. Remove
   `simpled-version-<deployment>`. Behavior does not change.
2. **Status.** Public `/health`, `gateway.daemon` in the spec, tokens, the private
   read API, logs, the local mount, `simpled status`, `simpled logs` and
   `simpled wait`.
   - **Prerequisite (7.2):** Swarm configs and secrets instead of bind mounts. This
     can ship on its own, before or alongside this phase.
3. **ACME.** The state store, challenges, delivery to Traefik, renewal,
   import of `acme.json`, and lifting the `global` + `letsencrypt` rejection.
4. **Manage.** Restart, scale and rollback with the manage token.
5. **Agent mode.** A global per-node daemon for stats, prune and volume warnings.

---

## 9. Open questions

1. **Always on, or opt-in?** This draft runs the daemon in every Docker environment,
   because it replaces the version sidecar and later issues certificates. That means
   every environment now has a container with the Docker socket. The alternative is
   `gateway.daemon.enabled: false`, which brings back a static version answer with
   no health.
2. **Traefik provider details, to confirm in a spike:** that `providers.http` on
   v2.10 accepts PEM inline in `tls.certificates` (it should, since `certFile` is
   file-or-content), and how it behaves when the endpoint is down at startup.
3. **Health of excluded or zero-replica services.** These probably do not count
   toward health, but some services are scaled to 0 on purpose.
4. **Several deployments on one domain.** `/health` follows `/version`'s ownership
   rule. A shared domain that no deployment owns answers neither.
