# simpled server — plan

The control plane for simpled: accounts, organizations, teams, projects and roles, and monitoring of the
deployments a project's GitHub repositories describe.

Architecture sources:
- Backend, bundle and tests: `D:\active_projects\doejoHQ\HobbyExchange\backend` (.NET 10, Controller → Validation → Ops → Presenter, `Read<T>`/`Mutate<T>`/`UnitOfWork`, opaque sessions in `auth_sessions`).
- Frontend, TS client and e2e: `D:\active_projects\hobbyshopify\hobby-pro` (`frontend/` React 19 + Vite + Tailwind v4 on subtree.js / trunk.js / operation-result.js, `backend-client/` hand-written Zod client, `e2e/` vitest).

## Layout

```
server/
├── CLAUDE.md  justfile  package.json          npm workspace: backend-client, e2e, web-app (from step 3)
├── appspec.yaml  localenv.yaml  local-infra.yaml  testspec.yaml  secrets.example/
├── docs/                                      plan.md, guides/ (backend, backend-testing, backend-client, e2e, frontend)
├── backend_services/                          .NET solution
│   ├── backend.sln  global.json  Directory.Build.props  .editorconfig  BannedSymbols.txt  justfile
│   ├── image/Dockerfile                       base build image
│   ├── config/                                CommonConfiguration, ConfigurationLoader (env + secret files)
│   ├── db/Entities/                           *Entity POCOs, snake_case [Table]/[Column]
│   ├── primary-db/                            PrimaryDbContext, DesignTimeDbContextFactory, Migrations
│   ├── db-setup/                              job: migrate
│   ├── core/                                  Auth, Libs (EntityAccess, MVCValidation), Primitive, Operations,
│   │                                          Validations, Presentation, Services (Clock, Mailer)
│   ├── emails/                                email_confirmation, reset_password, org_invitation templates
│   ├── main-svc/                              the Web API host
│   └── utest/                                 xUnit + EF InMemory
├── backend-client/                            @simpled/backend-client: TS source, Zod models, typed Result errors, MainClient
├── e2e/                                       vitest through backend-client + mock mailer; also drives web-app controllers
└── web-app/                                   React SPA (subtree.js: model + controller + page + route per page)
```

Infra: Postgres 17 and the SendGrid mock mailer. No Redis: confirmation tokens live in a table, rate limiting is in-process.

Local gateway `localhost:7080`: `/api` → main-svc (prefix stripped), `/` → web-app (nginx; the Vite dev server on :7000 under `just infra`). The SPA and API share an origin, so the browser uses an HttpOnly session cookie. The future CLI uses a bearer token on the same `auth_sessions` table (handler already supports both).

## Data model

| Table | Fields |
|---|---|
| `accounts` | login (email, lower-cased), name, password hash, confirmed, failed_login_count, locked_until |
| `auth_sessions` | copied from HobbyExchange (token hash, transport, idle/absolute expiry, revoked_at) |
| `confirmation_tokens` | token hash, purpose (`email-confirmation` / `reset-password`), account_id, expires_at, used_at |
| `organizations` | external_id, name, slug (unique), created_at |
| `org_members` | org, account, role; unique (org, account) |
| `org_invitations` | org, email, role, token hash, invited_by, created_at, expires_at (7 days), accepted_at, revoked_at; a new invitation to the same email revokes the open one |
| `teams` | org, name; unique (org, name) |
| `team_members` | team, account, added_at; unique (team, account); removed with the org membership |
| `projects` | org, name, slug; unique (org, slug) |
| `project_teams` | project, team; unique (project, team) |
| `github_connections` | org, name (unique per org), github_login, token_ciphertext (AES-GCM, key from the `github_token_key` secret), created_at |
| `repositories` | project, github_connection, owner, name (unique per project), default_branch, synced_at, synced_revision, sync_error |
| `environments` | repository, name (the envspec's directory), path (unique per repository) |
| `deployments` | environment, name (unique per environment), application_name, version_constraint, url (primary host), repo_version (the appspec's), status (`unknown` / `up` / `down` / `mismatch`), status_since, live_version, live_deployed_at, checked_at, check_error, failure_count |
| `deployment_events` | deployment, at, kind (`version-changed` / `down` / `up` / `mismatch`), version, previous_version, details |

## Roles

One role per organization member: `owner`, `admin`, `manager`, `member`. A manager "is a member of" a project when they belong to a team linked to it.

| Action | Owner | Admin | Manager | Member |
|---|---|---|---|---|
| Rename or delete the organization | ✓ | | | |
| Grant or revoke `owner` | ✓ | | | |
| Invite or remove members; set `admin`/`manager`/`member` | ✓ | ✓ (never on an owner) | | |
| Leave the organization | ✓ (not the last owner) | ✓ | ✓ | ✓ |
| Create teams and projects | ✓ | ✓ | ✓ (joins the team they create; a new project links only their teams, at least one) | |
| Delete a team or project | ✓ | ✓ | only their own teams / projects | |
| Manage a team's members | ✓ | ✓ | only teams they belong to | |
| Link or unlink a team and a project | ✓ | ✓ | only when they belong to the team and to the project | |
| View the org, members, teams and projects | ✓ | ✓ | ✓ | ✓ |
| Add, rotate or remove GitHub connections | ✓ | ✓ | | |
| Link, sync or unlink a project's repositories | ✓ | ✓ | only their own projects | |
| View connections (no tokens), repositories, deployments and their history | ✓ | ✓ | ✓ | ✓ |
| Restart a deployment | — | — | — | later |

An organization always keeps at least one owner. The rules are one pure module (`core/Rules/OrgPermissions`) used by validations, and presenters return the caller's role and allowed actions, so the UI hides what it can't do.

## Flows and endpoints (main-svc)

Auth (`[AllowAnonymous]`, rate limited, same answer whether or not the account exists):
- `POST Auth/Register` {email, password, name, organizationName} creates an unconfirmed account plus the organization (caller = owner) and mails a confirmation link.
- `POST Auth/ConfirmEmail` {token} confirms and signs in. `POST Auth/ResendConfirmation` {email}.
- `POST Auth/Login`, `POST Auth/Logout`, `POST Auth/RequestPasswordReset`, `POST Auth/ResetPassword`.
- `GET Me` returns the profile and the caller's organizations with their roles.

Organizations:
- `POST Orgs` creates another organization (caller = owner). `GET Orgs/{orgId}` (role + permissions), `PUT Orgs/{orgId}` rename, slug stays (owner), `DELETE Orgs/{orgId}` (owner; cascades).
- `GET Orgs/{orgId}/Members` (with `canChangeRole`/`canRemove` per row), `PUT Orgs/{orgId}/Members/{accountId}/Role` {role}, `DELETE Orgs/{orgId}/Members/{accountId}` (also leave). `accountId` is the account's external GUID; org/team/project/invitation ids are ints.

Invitations:
- `POST Orgs/{orgId}/Invitations` {email, role} mails a link to `invitation?token=`. `GET Orgs/{orgId}/Invitations` (open ones, expired flagged), `DELETE Orgs/{orgId}/Invitations/{id}` revokes. Owners and admins.
- Tokens travel in bodies, not paths, so they stay out of access logs:
  - `POST Invitations/Preview` {token} (anonymous) shows the org name, role, inviter and whether the email already has an account.
  - `POST Invitations/Accept` {token} for a signed-in account with that email.
  - `POST Invitations/Register` {token, name, password} creates a confirmed account (the link proves the email), joins the org and signs in.

Teams and projects:
- `GET|POST Orgs/{orgId}/Teams`, `GET|PUT|DELETE Teams/{teamId}`, `POST|DELETE Teams/{teamId}/Members/{accountId}`.
- `GET|POST Orgs/{orgId}/Projects` {name, slug, teamIds}, `GET|PUT|DELETE Projects/{projectId}`, `POST|DELETE Projects/{projectId}/Teams/{teamId}`.

Role checks are validations (`OrgValidation.TryGetMembership(orgId, claims.AccountId, vr, …)` then `EnsureAllowed(permission, …)`), because roles are per organization and can't be global policies. An organization a caller doesn't belong to answers `doesnt-exist`; a forbidden action answers `forbidden`.

Monitoring:
- `GET|POST Orgs/{orgId}/GitHubConnections` {name, token}: the token is checked with GitHub (`GET /user`) and its
  login kept. `PUT GitHubConnections/{connectionId}` {name, token?}, `DELETE GitHubConnections/{connectionId}`
  (refused with `incorrect-state` while repositories use it).
- `GET|POST Projects/{projectId}/Repositories` {connectionId, fullName}: `fullName` is `owner/name` or a GitHub URL;
  linking reads the default branch right away. `GET|DELETE Repositories/{repositoryId}`,
  `POST Repositories/{repositoryId}/Sync`. `GET Deployments/{deploymentId}/Events` (newest 50).

## Monitoring

A sync (`RepositoryOps.Sync`) lists the default branch's tree, reads every `envspec.yaml` (an environment, named
after its directory) and `appspec.yaml` (application name → version), and makes the rows match: a deployment's URL
is the first domain of its `primary_host` (`http` when `gateway.tls.disable`), `extends` is followed, templated
hosts (`$port(...)`) give none. An envspec that cannot be read leaves its environment as it was; problems go to
`sync_error`. Linking and "Sync now" sync in main-svc; `monitor-svc` re-syncs every
`REPOSITORY_SYNC_INTERVAL_SECONDS` (600).

`monitor-svc` (one instance) asks each deployment's `/.well-known/simpled/version` every `MONITOR_INTERVAL_SECONDS`
(60) and applies `MonitorOps.ApplyCheck`: an answer for the deployment is `up` and records a `version-changed`
event when the version moves; an answer for another application or deployment is `mismatch`; two failures in a row
are `down`. Status changes are events, except a new deployment's first `up`.

Next: deploy CI runs from GitHub Actions (workflows that run `simpled prepare-deployment <deployment>`), and
service health through a gateway endpoint simpled aggregates from the appspec healthchecks.

## web-app pages

`sign-in`, `register`, `check-email`, `confirm-email`, `forgot-password`, `reset-password`, `invitation` (accept or register), then the signed-in shell with an organization switcher: `org-overview`, `members` (roles, invitations), `teams`, `team` (members, projects), `projects`, `project` (deployments by repository and environment, linked teams), `github` (connections), `org-settings`, `new-org`.

## Build order

1. ✅ Backend scaffold: solution, config, auth and sessions, accounts (register/confirm/login/reset), mailer, db-setup, bundle specs; `just infra` + `just run` work.
2. ✅ Organizations, members, invitations, teams, projects, roles + `utest`.
3. ✅ `backend-client` + `e2e` (mock mailer for tokens; `simpled test e2e` on a fresh stack).
4. ✅ `web-app`, its controller tests, and e2e cases driving its controllers (`e2e/test/web-app/`).
5. ✅ CLAUDE.md and guides adapted to this repo (`docs/guides/`).
6. ✅ Monitoring: GitHub connections, linked repositories synced into environments and deployments, monitor-svc
   checking version endpoints, simpled-fakes for local runs and e2e, the `github` page and the project's
   deployments.
