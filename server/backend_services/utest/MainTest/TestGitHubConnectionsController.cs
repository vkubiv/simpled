using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Presentation;
using core.Primitive;
using db.Entities;
using main_svc.Controllers;
using Microsoft.AspNetCore.Mvc;
using primary_db;
using utest.TestSupport;
using utest.TestSupport.TestDoubles;

namespace utest.MainTest;

public class TestGitHubConnectionsController
{
    private static Task<ActionResult<GitHubConnectionPresentation>> Create(PrimaryDbContext db,
        OrganizationEntity organization, AccountEntity caller, FakeGitHub github, string name = "acme-bot",
        string token = MonitorSupport.Token) =>
        new GitHubConnectionsController().Create(organization.Id,
            new CreateGitHubConnectionRequest { Name = name, Token = token },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            github: github,
            cipher: MonitorSupport.Cipher,
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            mutateConnections: new Mutate<GitHubConnectionEntity>(db),
            readRepositories: new Read<RepositoryEntity>(db));

    private static Task<ActionResult<GitHubConnectionPresentation>> Update(PrimaryDbContext db,
        GitHubConnectionEntity connection, AccountEntity caller, FakeGitHub github, string name,
        string? token = null) =>
        new GitHubConnectionsController().Update(connection.Id,
            new UpdateGitHubConnectionRequest { Name = name, Token = token },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            github: github,
            cipher: MonitorSupport.Cipher,
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readConnections: new Read<GitHubConnectionEntity>(db),
            readRepositories: new Read<RepositoryEntity>(db));

    private static Task<ActionResult> Delete(PrimaryDbContext db, GitHubConnectionEntity connection,
        AccountEntity caller) =>
        new GitHubConnectionsController().Delete(connection.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            mutateConnections: new Mutate<GitHubConnectionEntity>(db),
            readRepositories: new Read<RepositoryEntity>(db));

    [Fact]
    public async Task TestCreate_KeepsTheLoginAndOnlyTheEncryptedToken()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);

        var result = await Create(db, organization, ann, MonitorSupport.GitHub());

        ControllerAssert.IsOkResult(result);
        Assert.Equal("octocat", result.Value!.GitHubLogin);
        var stored = Assert.Single(db.GitHubConnections);
        Assert.DoesNotContain(MonitorSupport.Token, stored.TokenCiphertext, StringComparison.Ordinal);
        Assert.Equal(MonitorSupport.Token, MonitorSupport.Cipher.Decrypt(stored.TokenCiphertext));
    }

    [Fact]
    public async Task TestCreate_TokenGitHubRefuses()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);

        ControllerAssert.HasValidationError(
            (await Create(db, organization, ann, MonitorSupport.GitHub(), token: "ghp_wrong")).Result,
            ErrorCode.IncorrectValue, "token");
        Assert.Empty(db.GitHubConnections);
    }

    [Fact]
    public async Task TestCreate_OwnersAndAdminsOnly()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "adam@test.com", OrgRole.Admin);
        var manager = await OrgSupport.AddMember(db, organization, "meg@test.com", OrgRole.Manager);

        ControllerAssert.HasValidationError((await Create(db, organization, manager, MonitorSupport.GitHub())).Result,
            ErrorCode.Forbidden, "role");
        ControllerAssert.IsOkResult(await Create(db, organization, admin, MonitorSupport.GitHub()));
    }

    [Fact]
    public async Task TestCreate_NameIsUniqueInTheOrganization()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        await MonitorSupport.CreateConnection(db, organization);

        ControllerAssert.HasValidationError((await Create(db, organization, ann, MonitorSupport.GitHub())).Result,
            ErrorCode.AlreadyExist, "name");
    }

    [Fact]
    public async Task TestUpdate_RenameKeepsTokenAndNewTokenIsChecked()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub().AddToken("ghp_new", "hubot");

        ControllerAssert.IsOkResult(await Update(db, connection, ann, github, "deploys"));
        Assert.Equal(MonitorSupport.Token, MonitorSupport.Cipher.Decrypt(connection.TokenCiphertext));

        ControllerAssert.HasValidationError((await Update(db, connection, ann, github, "deploys", "nope")).Result,
            ErrorCode.IncorrectValue, "token");

        var result = await Update(db, connection, ann, github, "deploys", "ghp_new");
        Assert.Equal("hubot", result.Value!.GitHubLogin);
        Assert.Equal("ghp_new", MonitorSupport.Cipher.Decrypt(connection.TokenCiphertext));
    }

    [Fact]
    public async Task TestUpdate_OtherOrganizationsConnectionDoesNotExist()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var (_, bob) = await OrgSupport.CreateOrg(db, "Other", "bob@test.com");
        var connection = await MonitorSupport.CreateConnection(db, organization);

        ControllerAssert.HasValidationError(
            (await Update(db, connection, bob, MonitorSupport.GitHub(), "mine")).Result,
            ErrorCode.DoesntExist, "connectionId");
    }

    [Fact]
    public async Task TestDelete_RefusedWhileRepositoriesUseIt()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var project = await OrgSupport.CreateProject(db, organization, "Shop");
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub();
        MonitorSupport.AddShop(github);
        var repository = await MonitorSupport.LinkRepository(db, project, connection, github);

        ControllerAssert.HasValidationError(await Delete(db, connection, ann), ErrorCode.IncorrectState,
            "connectionId");

        db.Repositories.Remove(repository);
        await db.SaveChangesAsync();
        ControllerAssert.IsOkResult(await Delete(db, connection, ann));
        Assert.Empty(db.GitHubConnections);
    }
}
