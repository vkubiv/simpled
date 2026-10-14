# Backend testing guide

Unit tests for `backend_services`, in `utest`. Run them with `just test` (or `just check-code` to build first).
Behaviour across the HTTP boundary and against real Postgres is the e2e suite's job ([e2e.md](e2e.md)).

## Setup

- **xUnit**, `[Fact]` / `[Theory]`, `async Task` tests.
- **EF InMemory**: `InMemoryPrimaryDb.Create()` gives every test its own database.
- **No mocking framework.** Hand-written doubles in `TestSupport/TestDoubles` (`RecordingMailer`).
- **Controllers are built with `new`** and their actions called with every dependency passed by name.

## Layout

```
utest/
  Core/Operations/      TestOrganizationOps, TestConfirmationTokenOps, TestAuthSessionOps
  Core/Validations/     TestAccountValidation
  Core/Rules/           TestOrgPermissions (theories over the role matrix)
  MainTest/             one Test<Controller> per controller
  TestSupport/          InMemoryPrimaryDb, CallContextSupport, AccountSupport, OrgSupport,
                        ControllerAssert, VRAssert, ControllerHelper, TestDoubles/RecordingMailer
```

Naming: `Test<Subject>` classes, `Test<Action>_<Scenario>` methods (`TestSetRole_LastOwnerStays`).

## Helpers

| Helper | Gives |
|---|---|
| `CallContextSupport.Main(db)` | a `MainCallContext` over the in-memory database |
| `AccountSupport.CreateAccount(db, email, confirmed)` | an account with password `AccountSupport.Password` |
| `OrgSupport.CreateOrg(db, name, ownerEmail)` | an organization and its owner |
| `OrgSupport.AddMember(db, org, email, role)` | a new account joined with a role |
| `OrgSupport.CreateTeam(db, org, name, members...)`, `CreateProject(db, org, name, teams...)` | teams and projects |
| `OrgSupport.Claims(account)` | the `MainClaims` a signed-in request would carry |
| `OrgSupport.RoleOf(db, org, account)` | the stored role |
| `RecordingMailer` | records mails; `.UserMailer()` for controllers, `.LastToken(email, template)` reads the link's token |
| `ControllerAssert.HasValidationError(result, code, field)`, `IsOkResult(result)` | assertions on action results |

Add a helper to `TestSupport` when a second test needs the same setup.

## A controller test

```csharp
public class TestTeamsController
{
    private static Task<ActionResult> AddMember(PrimaryDbContext db, TeamEntity team, AccountEntity caller,
        AccountEntity target) =>
        new TeamsController().AddMember(team.Id, target.ExternalId,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            mutateTeamMembers: new Mutate<TeamMemberEntity>(db));

    [Fact]
    public async Task TestAddMember_MustBeInOrganization()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var stranger = await AccountSupport.CreateAccount(db, "bob@test.com");
        var team = await OrgSupport.CreateTeam(db, organization, "Backend");

        ControllerAssert.HasValidationError(await AddMember(db, team, ann, stranger), ErrorCode.DoesntExist,
            "accountId");
    }
}
```

- A private static wrapper per action keeps the long `[FromServices]` list in one place per test class.
- A controller that reads `Request` / `Response` (sign-in, cookies) needs
  `controller.ControllerContext.HttpContext = new DefaultHttpContext()`.
- Assert on the stored state (`db.OrgMembers`, `db.Teams`) and on the error's code and field, not on sentences.
- For role rules, cover the allowed and the refused caller in one test (`TestManagerManagesOnlyOwnTeams`).

## What InMemory does not do

- **Unique indexes are not enforced.** A duplicate is caught by our `Ensure…` validations, which is what the
  tests check; the index is the database's backstop.
- **Cascades only reach tracked rows.** Deleting an organization removes its members in a test because they were
  loaded; Postgres removes everything. Cascade behaviour is covered end to end.
- **No transactions.** `UnitOfWork.ExecuteInTransaction` runs the body directly.
- The same `DbContext` serves every `Read`/`Mutate` in a test, so an entity changed and not saved is still visible
  to the next query. Validate before changing state (see `AuthController.ResetPassword`).
