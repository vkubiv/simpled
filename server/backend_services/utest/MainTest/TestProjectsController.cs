using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Presentation;
using core.Primitive;
using db.Entities;
using main_svc.Controllers;
using Microsoft.AspNetCore.Mvc;
using primary_db;
using utest.TestSupport;

namespace utest.MainTest;

public class TestProjectsController
{
    private static Task<ActionResult<ProjectPresentation>> Create(PrimaryDbContext db,
        OrganizationEntity organization, AccountEntity caller, string name, string? slug = null,
        params TeamEntity[] teams) =>
        new ProjectsController().Create(organization.Id,
            new CreateProjectRequest { Name = name, Slug = slug, TeamIds = teams.Select(t => t.Id).ToList() },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            mutateProjects: new Mutate<ProjectEntity>(db),
            mutateProjectTeams: new Mutate<ProjectTeamEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    private static Task<ActionResult<ProjectPresentation>> Update(PrimaryDbContext db, ProjectEntity project,
        AccountEntity caller, string name, string slug) =>
        new ProjectsController().Update(project.Id, new UpdateProjectRequest { Name = name, Slug = slug },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    private static Task<ActionResult> LinkTeam(PrimaryDbContext db, ProjectEntity project, TeamEntity team,
        AccountEntity caller) =>
        new ProjectsController().LinkTeam(project.Id, team.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            mutateProjectTeams: new Mutate<ProjectTeamEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    private static Task<ActionResult> UnlinkTeam(PrimaryDbContext db, ProjectEntity project, TeamEntity team,
        AccountEntity caller) =>
        new ProjectsController().UnlinkTeam(project.Id, team.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            mutateProjectTeams: new Mutate<ProjectTeamEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    private static Task<ActionResult> Delete(PrimaryDbContext db, ProjectEntity project, AccountEntity caller) =>
        new ProjectsController().Delete(project.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            mutateProjects: new Mutate<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    [Fact]
    public async Task TestCreate_SlugFromNameOrGiven()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);

        var first = (await Create(db, organization, ann, "Web Shop")).Value!;
        var second = (await Create(db, organization, ann, "Web Shop")).Value!;
        var given = (await Create(db, organization, ann, "Api", "api-v2")).Value!;

        Assert.Equal(["web-shop", "web-shop-2", "api-v2"], new[] { first.Slug, second.Slug, given.Slug });
        ControllerAssert.HasValidationError((await Create(db, organization, ann, "Other", "api-v2")).Result,
            ErrorCode.AlreadyExist, "slug");
        ControllerAssert.HasValidationError((await Create(db, organization, ann, "Other", "Bad Slug")).Result,
            ErrorCode.IncorrectValue, "slug");
    }

    [Fact]
    public async Task TestCreate_ManagerLinksOwnTeam()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var manager = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);
        var member = await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);
        var own = await OrgSupport.CreateTeam(db, organization, "Backend", manager);
        var other = await OrgSupport.CreateTeam(db, organization, "Frontend");

        ControllerAssert.HasValidationError((await Create(db, organization, manager, "Api")).Result,
            ErrorCode.Forbidden, "teamIds");
        ControllerAssert.HasValidationError((await Create(db, organization, manager, "Api", null, own, other))
            .Result, ErrorCode.Forbidden, "teamIds");
        ControllerAssert.HasValidationError((await Create(db, organization, member, "Api", null, own)).Result,
            ErrorCode.Forbidden);

        var project = (await Create(db, organization, manager, "Api", null, own)).Value!;
        Assert.Equal(["Backend"], project.Teams.Select(t => t.Name));
        Assert.True(project.IsMember);
        Assert.True(project.CanManage);
    }

    [Fact]
    public async Task TestCreate_TeamOfOtherOrganizationDoesntExist()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var (otherOrganization, _) = await OrgSupport.CreateOrg(db, "Globex", "zed@test.com");
        var foreignTeam = await OrgSupport.CreateTeam(db, otherOrganization, "Backend");

        ControllerAssert.HasValidationError((await Create(db, organization, ann, "Api", null, foreignTeam)).Result,
            ErrorCode.DoesntExist, "teamIds");
    }

    [Fact]
    public async Task TestManagerManagesOnlyProjectsOfTheirTeams()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var manager = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);
        var own = await OrgSupport.CreateTeam(db, organization, "Backend", manager);
        var other = await OrgSupport.CreateTeam(db, organization, "Frontend");
        var ownProject = await OrgSupport.CreateProject(db, organization, "Api", own);
        var otherProject = await OrgSupport.CreateProject(db, organization, "Web", other);

        ControllerAssert.HasValidationError((await Update(db, otherProject, manager, "Web", "web")).Result,
            ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await Delete(db, otherProject, manager), ErrorCode.Forbidden);
        // Linking needs both: their project with someone else's team, or their team with someone else's project.
        ControllerAssert.HasValidationError(await LinkTeam(db, ownProject, other, manager), ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await LinkTeam(db, otherProject, own, manager), ErrorCode.Forbidden);

        var updated = (await Update(db, ownProject, manager, "Api v2", "api-v2")).Value!;
        Assert.Equal(("Api v2", "api-v2"), (updated.Name, updated.Slug));
        ControllerAssert.IsOkResult(await Delete(db, ownProject, manager));
        Assert.DoesNotContain(db.Projects, p => p.Id == ownProject.Id);
    }

    [Fact]
    public async Task TestLinkAndUnlinkTeam()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var team = await OrgSupport.CreateTeam(db, organization, "Backend");
        var project = await OrgSupport.CreateProject(db, organization, "Api");

        ControllerAssert.IsOkResult(await LinkTeam(db, project, team, ann));
        ControllerAssert.HasValidationError(await LinkTeam(db, project, team, ann), ErrorCode.AlreadyExist,
            "teamId");
        ControllerAssert.IsOkResult(await UnlinkTeam(db, project, team, ann));
        ControllerAssert.HasValidationError(await UnlinkTeam(db, project, team, ann), ErrorCode.DoesntExist,
            "teamId");
        Assert.Empty(db.ProjectTeams);
    }

    [Fact]
    public async Task TestGetProjects_MemberSeesAllWithoutManage()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var member = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);
        var team = await OrgSupport.CreateTeam(db, organization, "Backend", member);
        await OrgSupport.CreateProject(db, organization, "Web");
        await OrgSupport.CreateProject(db, organization, "Api", team);

        var result = await new ProjectsController().GetProjects(organization.Id, OrgSupport.Claims(member),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readProjects: new Read<ProjectEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

        var projects = result.Value!;
        Assert.Equal(["Api", "Web"], projects.Select(p => p.Name));
        Assert.True(projects[0].IsMember);
        Assert.False(projects[1].IsMember);
        Assert.All(projects, p => Assert.False(p.CanManage));
    }
}
