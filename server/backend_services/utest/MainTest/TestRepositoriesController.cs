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

public class TestRepositoriesController
{
    private static Task<ActionResult<RepositoryPresentation>> Link(PrimaryDbContext db, ProjectEntity project,
        AccountEntity caller, GitHubConnectionEntity connection, FakeGitHub github, string fullName = "acme/shop") =>
        new RepositoriesController().Link(project.Id,
            new LinkRepositoryRequest { ConnectionId = connection.Id, FullName = fullName },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            github: github,
            cipher: MonitorSupport.Cipher,
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            readConnections: new Read<GitHubConnectionEntity>(db),
            mutateRepositories: new Mutate<RepositoryEntity>(db),
            mutateEnvironments: new Mutate<DeployEnvironmentEntity>(db),
            mutateDeployments: new Mutate<DeploymentEntity>(db));

    private static Task<ActionResult<List<RepositoryPresentation>>> GetRepositories(PrimaryDbContext db,
        ProjectEntity project, AccountEntity caller) =>
        new RepositoriesController().GetRepositories(project.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            readRepositories: new Read<RepositoryEntity>(db),
            readEnvironments: new Read<DeployEnvironmentEntity>(db),
            readDeployments: new Read<DeploymentEntity>(db));

    private static Task<ActionResult<RepositoryPresentation>> Sync(PrimaryDbContext db, RepositoryEntity repository,
        AccountEntity caller, FakeGitHub github) =>
        new RepositoriesController().Sync(repository.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            github: github,
            cipher: MonitorSupport.Cipher,
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            readRepositories: new Read<RepositoryEntity>(db),
            readConnections: new Read<GitHubConnectionEntity>(db),
            mutateEnvironments: new Mutate<DeployEnvironmentEntity>(db),
            mutateDeployments: new Mutate<DeploymentEntity>(db));

    private static Task<ActionResult> Unlink(PrimaryDbContext db, RepositoryEntity repository,
        AccountEntity caller) =>
        new RepositoriesController().Unlink(repository.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            mutateRepositories: new Mutate<RepositoryEntity>(db));

    private static async Task<(PrimaryDbContext Db, OrganizationEntity Organization, AccountEntity Owner,
        ProjectEntity Project, GitHubConnectionEntity Connection, FakeGitHub GitHub)> Setup()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var project = await OrgSupport.CreateProject(db, organization, "Shop");
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub();
        MonitorSupport.AddShop(github);
        return (db, organization, ann, project, connection, github);
    }

    [Theory]
    [InlineData("acme/shop")]
    [InlineData("https://github.com/acme/shop")]
    [InlineData("github.com/ACME/shop.git")]
    public async Task TestLink_SyncsRightAway(string fullName)
    {
        var (db, _, ann, project, connection, github) = await Setup();

        var result = await Link(db, project, ann, connection, github, fullName);

        ControllerAssert.IsOkResult(result);
        var repository = result.Value!;
        Assert.Equal("acme", repository.Owner);
        Assert.Equal("shop", repository.Name);
        Assert.NotNull(repository.SyncedAt);
        var environment = Assert.Single(repository.Environments);
        Assert.Equal("prod", environment.Name);
        Assert.Equal(["canary", "prod"], environment.Deployments.Select(d => d.Name));
        Assert.All(environment.Deployments, d => Assert.Equal("1.2.3", d.RepoVersion));
    }

    [Fact]
    public async Task TestLink_RepositoryTheTokenCannotSee()
    {
        var (db, _, ann, project, connection, github) = await Setup();
        github.AddRepository("acme", "secret", "ghp_other");

        ControllerAssert.HasValidationError((await Link(db, project, ann, connection, github, "acme/secret")).Result,
            ErrorCode.DoesntExist, "fullName");
        ControllerAssert.HasValidationError((await Link(db, project, ann, connection, github, "not a repo")).Result,
            ErrorCode.IncorrectValue, "fullName");
        Assert.Empty(db.Repositories);
    }

    [Fact]
    public async Task TestLink_OnceAndWithTheOrganizationsConnection()
    {
        var (db, _, ann, project, connection, github) = await Setup();
        var (otherOrganization, _) = await OrgSupport.CreateOrg(db, "Other", "bob@test.com");
        var otherConnection = await MonitorSupport.CreateConnection(db, otherOrganization);

        ControllerAssert.HasValidationError((await Link(db, project, ann, otherConnection, github)).Result,
            ErrorCode.DoesntExist, "connectionId");

        ControllerAssert.IsOkResult(await Link(db, project, ann, connection, github));
        ControllerAssert.HasValidationError((await Link(db, project, ann, connection, github)).Result,
            ErrorCode.AlreadyExist, "fullName");
    }

    [Fact]
    public async Task TestLink_ManagerOfTheProjectOnly()
    {
        var (db, organization, _, project, connection, github) = await Setup();
        var meg = await OrgSupport.AddMember(db, organization, "meg@test.com", OrgRole.Manager);
        var otherProject = await OrgSupport.CreateProject(db, organization, "Blog");
        var team = await OrgSupport.CreateTeam(db, organization, "Shop team", meg);
        db.ProjectTeams.Add(new ProjectTeamEntity { ProjectId = project.Id, TeamId = team.Id });
        await db.SaveChangesAsync();

        ControllerAssert.HasValidationError((await Link(db, otherProject, meg, connection, github)).Result,
            ErrorCode.Forbidden, "role");
        ControllerAssert.IsOkResult(await Link(db, project, meg, connection, github));
    }

    [Fact]
    public async Task TestGetRepositories_MembersSeeButCannotManage()
    {
        var (db, organization, _, project, connection, github) = await Setup();
        await MonitorSupport.LinkRepository(db, project, connection, github);
        var max = await OrgSupport.AddMember(db, organization, "max@test.com", OrgRole.Member);
        var (_, stranger) = await OrgSupport.CreateOrg(db, "Other", "bob@test.com");

        var repositories = (await GetRepositories(db, project, max)).Value!;
        Assert.False(Assert.Single(repositories).CanManage);

        ControllerAssert.HasValidationError((await GetRepositories(db, project, stranger)).Result,
            ErrorCode.DoesntExist, "projectId");
    }

    [Fact]
    public async Task TestSyncAndUnlink()
    {
        var (db, organization, ann, project, connection, github) = await Setup();
        var repository = await MonitorSupport.LinkRepository(db, project, connection, github);
        var max = await OrgSupport.AddMember(db, organization, "max@test.com", OrgRole.Member);

        ControllerAssert.HasValidationError((await Sync(db, repository, max, github)).Result, ErrorCode.Forbidden,
            "role");
        ControllerAssert.IsOkResult(await Sync(db, repository, ann, github));

        ControllerAssert.HasValidationError(await Unlink(db, repository, max), ErrorCode.Forbidden, "role");
        ControllerAssert.IsOkResult(await Unlink(db, repository, ann));
        Assert.Empty(db.Repositories);
    }
}
