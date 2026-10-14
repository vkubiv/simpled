# Backend guide

How `server/backend_services` is organized and how a feature is added to it. Read with
[backend-testing.md](backend-testing.md). Adapted from the HobbyExchange backend, which this one copies.

## 1. Map

A .NET 10 solution, `backend_services/backend.sln`:

| Project | What it holds |
|---|---|
| `main-svc` | The Web API host: `Program.cs` (DI, auth, rate limiting, CORS, Swagger, `/health`), `Controllers/`, `Auth/` (`MainClaims`, `MainAuth`, `SessionSignIn`), `MainCallContext`. |
| `core` | Everything else: `Operations/`, `Validations/`, `Presentation/`, `Rules/`, `Primitive/`, `Auth/` (sessions, cookie, rate limiting), `Libs/` (`EntityAccess`, `MVCValidation`), `Services/` (`Clock`, `Mailer`). |
| `db` | `Entities/*Entity.cs`: POCOs with snake_case `[Table]` / `[Column]` and indexes as attributes. |
| `primary-db` | `PrimaryDbContext` (the `DbSet`s), `DesignTimeDbContextFactory`, `Migrations/`. |
| `db-setup` | The `job` that applies migrations before main-svc starts. |
| `config` | `CommonConfiguration` + `ConfigurationLoader`: environment variables and secret files. Not `appsettings.json`. |
| `utest` | xUnit + EF InMemory. |
| `emails/en/` | Mail templates: first line the subject, the rest the HTML body, Stubble (`{{name}}`, `{{link}}`, ...). |

Tables: `accounts`, `auth_sessions`, `confirmation_tokens`, `organizations`, `org_members`, `org_invitations`,
`teams`, `team_members`, `projects`, `project_teams`. Deleting an organization, team or project cascades in the
database.

## 2. The request flow

Every action goes **Controller → Validation → Operation → Presentation**, then saves once:

```csharp
// main-svc/Controllers/TeamsController.cs
[HttpPost("Orgs/{orgId:int}/Teams")]
public async Task<ActionResult<TeamPresentation>> Create(
    int orgId,
    TeamRequest request,
    [FromServices] MainClaims claims,
    [FromServices] MainCallContext context,
    [FromServices] Read<OrgMemberEntity> readMembers,
    [FromServices] Read<OrganizationEntity> readOrganizations,
    [FromServices] Read<AccountEntity> readAccounts,
    [FromServices] Mutate<TeamEntity> mutateTeams,
    [FromServices] Mutate<TeamMemberEntity> mutateTeamMembers,
    [FromServices] Read<ProjectTeamEntity> readProjectTeams,
    [FromServices] Read<ProjectEntity> readProjects
)
{
    using var vr = new ValidationResult();
    var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
    if (access == null ||
        !OrgValidation.EnsureAllowed(OrgPermissions.CanCreateTeam(access.Role), vr) ||
        !TeamValidation.ValidateName(request.Name, vr) ||
        !await TeamValidation.EnsureNameFree(orgId, request.Name, null, vr, mutateTeams.Read))
    {
        return vr.ReturnErrorResponse();
    }

    var now = context.Clock.Now;
    var team = await TeamOps.Create(orgId, request.Name, now, mutateTeams);
    if (access.Role == OrgRole.Manager)
    {
        var account = (await AccountOps.TryGetAccountById(claims.AccountId, readAccounts))!;
        await TeamOps.AddMember(team, account, now, mutateTeamMembers);
    }

    await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
    return await TeamPresenter.PresentTeam(team, access, mutateTeamMembers.Read, readAccounts, readMembers,
        readProjectTeams, readProjects, context.CancellationToken);
}
```

### Controllers

- Classic `[ApiController]`. Dependencies come per action through `[FromServices]`; no constructor injection,
  no minimal APIs.
- Request models live in the controller file (`TeamRequest`), or come from `core/Primitive` when one fits
  (`Registration`).
- Routes are attributes on the actions: `Orgs/{orgId:int}/Teams` next to `Teams/{teamId:int}`.
- `context.UnitOfWork.SaveChangesAsync` once, at the end. An action that must save earlier says why
  (`InvitationsController.Register` saves the account so the session row can reference its id).

### `MainCallContext`

`CallContext` (`CancellationToken`, `Logger`, `Clock`) plus the `UnitOfWork`. Controllers take it; ops take
the clock's `now` as a parameter rather than the context, so they stay trivially testable.

### Entity access

- `Read<T>` is an `IQueryable<T>`; `Mutate<T>` adds, removes and tracks, and its `.Read` queries the same set.
  Both are registered as open generics, so a new entity needs no DI line.
- No `DbContext` outside `primary-db` and the `EntityAccess` lib. No repositories.
- EF's `Include` is banned (`BannedSymbols.txt`). Use `core/Libs/EntityAccess/IncludeExtensions`, which takes
  the joined entity's `Read<T>`: `readMembers.Include(m => m.Organization, readOrganizations)`.
- Entities read through `Read<T>` / `Mutate<T>` are tracked; changing a property and saving is the update.

### Validations (`core/Validations`)

Static methods in `[Feature]Validation` classes that write into the `ValidationResult`:

| Shape | Returns | Example |
|---|---|---|
| `Validate…` | `bool` | `ProjectValidation.ValidateSlug(slug, vr, required: false)` |
| `Ensure…` | `bool` | `TeamValidation.EnsureNameFree(...)`, `OrgValidation.EnsureNotLastOwner(...)` |
| `TryGet…` | entity or `null` | `TeamValidation.TryGetTeam(...)`, `OrgValidation.TryGetMember(...)` |

Chain them with `||` and return `vr.ReturnErrorResponse()` when one fails. Decide on the return values, never on
`vr.GetErrors().Count`. A `ValidationResult` with errors that is disposed without `ReturnErrorResponse` throws.

Error codes (`ErrorCode`, kebab-case): `value-required`, `incorrect-value`, `incorrect-state`, `already-exist`,
`doesnt-exist`, `forbidden`, plus `email-not-confirmed` from login. `incorrectValue` names the field or route
parameter (`email`, `orgId`, `teamIds`), which is how `backend-client` maps errors to classes; keep names stable.

### Operations (`core/Operations`)

Static methods on static `[Feature]Ops` classes, stateless, every dependency a parameter. Callers pass them by
name: `OrganizationOps.Create(name: ..., owner: ..., now: ..., mutateOrganizations: ..., mutateMembers: ...)`.
They change tracked entities and add rows; they do not save. `TryGet…` ops return `null` for a miss; turning that
into an error is the validation's job.

### Presentation (`core/Presentation`)

`[Feature]Presenter.Present…` builds the response classes (`…Presentation`), joining what it needs. It also
answers *what the caller may do* (`OrgPermissionsPresentation`, `canManage`, `canChangeRole`), so the web app
shows only what main-svc would accept. Account ids go out as the account's `ExternalId` GUID; organization, team,
project and invitation ids are ints.

## 3. Auth

- **Sessions.** Opaque tokens in `auth_sessions`, only their SHA-256 stored (`AuthSessionOps`). The web app gets
  an HttpOnly `session` cookie (SameSite Lax); `?useToken=true` on a sign-in returns the token in the body for the
  CLI, which sends it as a bearer. `MainAuth` holds both policies (cookie 12 h idle / 7 d absolute, token 30 d /
  90 d). `MainAuthenticationHandler` rebuilds `MainClaims` from the database on every request and refuses an
  unconfirmed account.
- **Every endpoint needs a session** (fallback policy). `[AllowAnonymous]` goes on the action, never the
  controller. Anonymous endpoints that take an email or send mail carry
  `[EnableRateLimiting(AuthRateLimiting.Policy)]` and answer the same whether or not the account exists.
- Signing in goes through `SessionSignIn.SignIn(this, account, useToken, ...)`.
- Logins go through `AccountOps.TryLogin` (5 failures lock the account for 15 minutes; save on the failure path
  too). Every password that is set is checked with `AccountValidation.ValidatePassword`. A password reset revokes
  every session (`AuthSessionOps.RevokeAllSessions`).
- **Emailed links.** Single-use tokens, hash stored: `ConfirmationTokenOps` for email confirmation and password
  reset (a new token voids the older ones of its purpose), `OrgInvitationOps` for invitations (the hash is on the
  invitation row). Links point at web app pages under `WEB_APP_URL` (`UserMailer`). Tokens travel in request
  bodies, never in API paths.

## 4. Organizations and roles

One role per member: `owner`, `admin`, `manager`, `member` (`core/Primitive/OrgRole`). The rules are one pure
module, `core/Rules/OrgPermissions` (`CanManageOrganization`, `CanManageMembers`, `AssignableRoles`,
`CanChangeRole`, `CanRemoveMember`, `CanCreateTeam`, `CanCreateProject`, `CanManageTeam(role, isTeamMember)`,
`CanManageProject(role, isProjectMember)`); `docs/plan.md` has the matrix. A rule that needs data stays in
validations (`OrgValidation.EnsureNotLastOwner`).

An action on an organization's data starts from the caller's access:

| Route | Access |
|---|---|
| `Orgs/{orgId}/…` | `OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, …)` → `OrgAccess(Organization, Member)` with `.Role` |
| `Teams/{teamId}/…` | `TeamValidation.TryGetTeamAccess(…)` → `(Team, Access, IsTeamMember)` |
| `Projects/{projectId}/…` | `ProjectValidation.TryGetProjectAccess(…)` → `(Project, Access, IsProjectMember)` |

A caller outside the organization gets `doesnt-exist` on the route parameter, the same as a missing id. Then
`OrgValidation.EnsureAllowed(OrgPermissions.Can…(...), vr)` answers `forbidden`. A manager "is in" a project when
they are in one of its teams (`ProjectOps.IsMember`).

## 5. Adding a feature

1. **Entity** in `db/Entities` (snake_case `[Table]`/`[Column]`, indexes as attributes), its `DbSet` in
   `PrimaryDbContext`, then `just add-migration <Name>` from `server/` and read the generated migration.
2. **Rules**, if it adds a permission: a pure method in `OrgPermissions`, with a theory in
   `utest/Core/Rules/TestOrgPermissions`.
3. **Validation** in `core/Validations/[Feature]Validation.cs`, **operations** in `core/Operations/[Feature]Ops.cs`,
   **presenter** in `core/Presentation/[Feature]Presenter.cs`.
4. **Controller** in `main-svc/Controllers`, following the flow above.
5. **Tests** in `utest` (see [backend-testing.md](backend-testing.md)), then the endpoint in `backend-client`
   and a scenario in `e2e` ([backend-client.md](backend-client.md), [e2e.md](e2e.md)).
6. A new mail: a template in `emails/en/<name>` and a `UserMailer.Send…` method; send after the save.
7. A new setting: a property on `CommonConfiguration`, read in `ConfigurationLoader`, declared in `appspec.yaml`
   (`external` / `optional`) and given a local value in `localenv.yaml`. A new secret also goes in
   `secrets.example/`.

## 6. Commands

From `server/` (`just --list` for all):

```sh
just check-code          # restore, build, unit tests
just add-migration Name
just apply-migrations    # against `just infra`'s Postgres
just build               # docker images, needed by `just run` and `just test-e2e`
```

main-svc run from the IDE listens on `http://localhost:7001` and reads `backend_services/main-svc/.env`, which
`simpled local generate-config` (part of `just infra`) writes.

## 7. Standards

1. Async for all I/O; no `Async` suffix on our own methods.
2. Named parameters when calling ops.
3. Nullable reference types on; `!` only where a validation has just guaranteed the value.
4. Expected failures are `ValidationResult` errors; exceptions are for bugs and infrastructure failures.
5. Controllers do not query: lookups live in ops or validations.
6. Comments say why, not what; keep them few.
