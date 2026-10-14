# simpled server

Guidance for Claude Code in `server/`. Write few comments: they say why, not what.

The simpled control plane: accounts, organizations, teams, projects and roles, as one simpled bundle
(`appspec.yaml`). Plan, data model and role matrix: `docs/plan.md`.

| Part | Where | Guide |
|---|---|---|
| main-svc and db-setup (.NET 10) | `backend_services/` | `docs/guides/backend.md`, `docs/guides/backend-testing.md` |
| TypeScript client for main-svc | `backend-client/` | `docs/guides/backend-client.md` |
| End-to-end suite | `e2e/` | `docs/guides/e2e.md` |
| React web app | `web-app/` | `docs/guides/frontend.md` |

Read the guide for the part you change before changing it. The architecture comes from
`D:\active_projects\doejoHQ\HobbyExchange` (backend) and `D:\active_projects\hobbyshopify\hobby-pro` (client,
e2e, frontend); the guides are self-contained, look there only for an example they don't have.

## Rules that are easy to break

- Backend: Controller → Validation → Ops → Presenter, `[FromServices]` per action, one `SaveChangesAsync` at the
  end, `Read<T>`/`Mutate<T>` only, EF `Include` banned (use `IncludeExtensions`), named parameters on ops calls.
- Role rules live only in `core/Rules/OrgPermissions`; actions start from `OrgValidation.TryGetAccess` /
  `TeamValidation.TryGetTeamAccess` / `ProjectValidation.TryGetProjectAccess`, then `EnsureAllowed`. Presenters
  return the same answers as flags, and the web app hides what they deny.
- Error codes are kebab-case and `incorrectValue` names the field or route parameter; `backend-client` maps on
  both, so renaming one is a client change too.
- Emailed tokens are single use, stored as hashes, and travel in request bodies, never URL paths of the API.
- Settings are env vars and secret files (`config/ConfigurationLoader.cs`), declared in `appspec.yaml` and given
  local values in `localenv.yaml`. Never a literal host port in the specs: use `$port(name)`.
- Client methods return `Result` with a precise error union; e2e tests go through the client (and the web app's
  controllers), never the database; web-app controllers have no DOM and no business logic (ops do), and views
  never touch repositories or ops.
- A change to an endpoint touches main-svc, `backend-client`, `utest` and `e2e` in the same change.

## Commands (from `server/`)

```sh
just secrets-init        # once per machine: ./secrets from secrets.example
just npm-install         # the npm workspace (backend-client, e2e, web-app)
just infra               # Postgres, mock mailer, gateway; then main-svc from the IDE and `just web-dev`
just apply-migrations    # against `just infra`'s Postgres
just web-dev             # Vite on :7000; open http://localhost:7080, where emailed links land
just build               # docker images (main-svc, db-setup, web-app)
just run                 # everything in Docker behind http://localhost:7080 (`/` web app, `/api` main-svc)
just check-code          # backend: build + unit tests
just check-ts            # TypeScript: tsc + biome + web-app unit tests
just test-e2e            # e2e on a fresh stack of its own (needs `just build`)
just test-e2e-attached   # e2e against the running stack
just add-migration Name
```

Named host ports (`localenv.yaml`): gateway 7080, web-app 7000, main-svc 7001, mock mailer 7025 (its UI lists
every email), Postgres 7054. Instance N (`--instance N`, `SIMPLED_INSTANCE`, or a `.simpled-instance` file) adds
N × 100 and runs as its own compose project with its own data. The IDE launch profile and Vite use instance 0's
ports. Infra services are `simpled-db` / `simpled-mailer` because simpled names instance-0 containers after
their services and other local projects own `primary-db` / `mock-mailer`.
