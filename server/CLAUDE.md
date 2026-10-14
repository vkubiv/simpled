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

## Commands (from `server/`)

```sh
just secrets-init        # once per machine: seeds ./secrets from secrets.example
just infra               # Postgres (5440) + mock mailer (http://localhost:3020) + gateway; run main-svc from the IDE
just apply-migrations    # after `just infra`, before running main-svc from the IDE
just run                 # full dockerized stack (needs `just build`); gateway http://localhost:7080, main-svc under /api
just build               # docker images
just check-code          # restore, build, unit tests
just add-migration Name
```

main-svc run from the IDE listens on http://localhost:7001 (`launchSettings.json`); the gateway forwards `/api` to it.
Infra service names are `simpled-db` / `simpled-mailer`, because simpled names containers after services and
other local projects already own `primary-db` / `mock-mailer`.

## Testing

`utest`: xUnit + EF InMemory, controllers instantiated with `new`, dependencies passed by named parameter,
hand-written test doubles (`RecordingMailer` captures emailed tokens), `OrgSupport` builds orgs, members, teams and projects. Naming `Test<Subject>` / `Test<Action>_<Scenario>`.
