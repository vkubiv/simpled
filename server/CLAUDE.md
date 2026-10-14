# simpled server

Guidance for Claude Code in `server/`. Write less comments.

The simpled control plane: accounts, organizations, teams, projects, roles. One simpled bundle (`appspec.yaml`)
with a .NET 10 backend in `backend_services/`. Plan and role model: `docs/plan.md`.

Architecture is copied from `D:\active_projects\doejoHQ\HobbyExchange\backend` (backend, see its
`docs/guides/backend.md` and `backend-testing.md`) and `D:\active_projects\hobbyshopify\hobby-pro`
(frontend, TS client, e2e).

## Backend rules

- Classic `[ApiController]` controllers, dependencies injected per action with `[FromServices]`. No minimal APIs, no constructor DI in controllers.
- Request flow: Controller → `core/Validations` (static `Ensure…` / `TryGet…` / `Validate…` with `ValidationResult`) → `core/Operations` (static `*Ops`, named parameters at call sites) → `core/Presentation` (`*Presenter`). `SaveChangesAsync` once at the end of the action.
- Data access only through `Read<T>` / `Mutate<T>` / `UnitOfWork`; no repositories, no MediatR, no AutoMapper. `Include` only via `core/Libs/EntityAccess/IncludeExtensions.cs` (EF's own is banned).
- Auth: opaque sessions in `auth_sessions` (hash only), HttpOnly cookie for the web app, `?useToken=true` + bearer for the CLI. Every endpoint needs a session unless the action has `[AllowAnonymous]`; anonymous auth endpoints are rate limited and answer the same whether or not the account exists.
- Role checks: `core/Rules/OrgPermissions` (pure) is the only place rules live. Controllers get access with `OrgValidation.TryGetAccess` / `TeamValidation.TryGetTeamAccess` / `ProjectValidation.TryGetProjectAccess` (non-members get `doesnt-exist`), then `OrgValidation.EnsureAllowed(OrgPermissions.Can…)` (`forbidden`). Presenters expose the same answers as `permissions` / `canManage` flags.
- Emailed links carry single-use tokens from `ConfirmationTokenOps` (hash stored in `confirmation_tokens`); link targets are web app pages under `WEB_APP_URL`.
- Config is env vars + secret files via `config/ConfigurationLoader.cs`; `appsettings.json` is not for settings.
- Adding an entity: `db/Entities/*Entity.cs` with snake_case `[Table]`/`[Column]`, `DbSet` in `PrimaryDbContext`, `just add-migration <Name>`.
- Error codes are kebab-case (`incorrect-value`, `email-not-confirmed`). Async methods have no `Async` suffix.

## TypeScript rules (`backend-client`, `e2e`, `web-app`)

- npm workspace at `server/` (`backend-client`, `e2e`, `web-app`). `backend-client` exports its
  `.ts` source, no build step. Copied from hobby-pro's `backend-client-guide.md`.
- Every API method takes `ctx: ApiContext` first and returns `Promise<Result<T, E>>` with `E` a union of concrete
  error classes; the failure mapper names what it maps and ends in `signedIn(e)` / `rateLimited(e)` or
  `throw new UnexpectedApiError(e)`. Write one mapper per method rather than a shared option bag, which widens the union.
- Models are Zod schemas in `src/model`, parsed with `parse()`; dates are `isoDate`.
- Contexts: browser → `browser` (cookie store), Node → `{ cookieJar: new MemoryCookieJar() }`, CLI → `withToken(token)`.
- `e2e` imports only from `@simpled/backend-client`, never mocks, gets emailed tokens from the mock mailer
  (`test/utils/mailbox.ts`) and builds scenarios with `Sugar` (per-run unique emails, people by alias).

## web-app

React 19 + Vite + Tailwind v4 on subtree.js / trunk.js / operation-result.js, copied from
`D:\active_projects\hobbyshopify\hobby-pro\frontend` (its `CLAUDE.md` is the operating manual; library docs in
`D:\apps\subtree.js\docs`). Layout: `src/core/` (auth, ops, repositories, primitives; no React), `src/shell/`
(org switcher + nav), `src/pages/<page>/` (model, controller, page, route, controller test), `src/components/`
(presentational), `src/testing/fakes.ts`.

- Every page: `State` of `Rx` fields + abstract `Actions`; controller with `Deps = Pick<AppDeps, …>` and a
  `Routing` of callbacks; the route component reads the URL once and builds it inside `<Subtree key deps>`.
- Calls go through ops (`core/ops/*`), which take one options object and return `AsyncResult`; controllers never
  mutate repositories. Repositories: `meRepository` (account + organizations), `orgRepository` (the organization the
  shell shows, with the caller's `permissions`); pages hide what those flags deny.
- Session: an HttpOnly cookie the app can't read. `AccountSession` holds the stage from `/Me`, sign-in and sign-out,
  and `sessionFetch` turns any 401 into signed out. Node (e2e) passes a `MemoryCookieJar` context instead.
- Emailed links land on `confirm-email`, `reset-password`, `invitation`; confirming is a click, not on load, because
  mail scanners open links and tokens are single use.
- Controllers must run without a DOM: e2e `test/web-app/` drives the real controllers against the stack
  (`test/utils/web-app.ts`). `window.confirm` and `useState` stay in views.

## Commands (from `server/`)

```sh
just secrets-init        # once per machine: seeds ./secrets from secrets.example
just infra               # Postgres (7054) + mock mailer (http://localhost:7025) + gateway; run main-svc from the IDE
just apply-migrations    # after `just infra`, before running main-svc from the IDE
just run                 # full dockerized stack (needs `just build`); gateway http://localhost:7080, main-svc under /api
just build               # docker images
just check-code          # restore, build, unit tests
just add-migration Name
just npm-install         # npm workspace
just check-ts            # tsc + biome (+ vitest for web-app) across the workspace
just web-dev             # Vite on :7000; with `just infra` open http://localhost:7080 (emailed links land there)
just test-e2e            # e2e on a fresh stack, first free instance (testspec.yaml); `just build` first
just test-e2e-attached   # e2e against what already runs on :7080
```

Host ports are named in `localenv.yaml` (`ports:`, written `$port(name)`): gateway 7080, web-app 7000, main-svc 7001,
mock mailer 7025, Postgres 7054. Instance N (`--instance N`, `SIMPLED_INSTANCE` or a `.simpled-instance` file) adds N * 100 and
runs as its own compose project with its own data, so stacks from several worktrees run side by side. Never write a
literal host port in the specs. main-svc run from the IDE listens on http://localhost:7001 (`launchSettings.json`), so
IDE runs are instance 0; the gateway forwards `/api` to it.
Infra service names are `simpled-db` / `simpled-mailer`, because simpled names containers after services and
other local projects already own `primary-db` / `mock-mailer`.

## Testing

`utest`: xUnit + EF InMemory, controllers instantiated with `new`, dependencies passed by named parameter,
hand-written test doubles (`RecordingMailer` captures emailed tokens), `OrgSupport` builds orgs, members, teams and projects. Naming `Test<Subject>` / `Test<Action>_<Scenario>`.
