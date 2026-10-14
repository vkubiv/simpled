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

public class TestTeamsController
{
    private static Task<ActionResult<TeamPresentation>> Create(PrimaryDbContext db, OrganizationEntity organization,
        AccountEntity caller, string name) =>
        new TeamsController().Create(organization.Id, new TeamRequest { Name = name },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateTeams: new Mutate<TeamEntity>(db),
            mutateTeamMembers: new Mutate<TeamMemberEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readProjects: new Read<ProjectEntity>(db));

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

    private static Task<ActionResult> RemoveMember(PrimaryDbContext db, TeamEntity team, AccountEntity caller,
        AccountEntity target) =>
        new TeamsController().RemoveMember(team.Id, target.ExternalId,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            mutateTeamMembers: new Mutate<TeamMemberEntity>(db));

    private static Task<ActionResult> Delete(PrimaryDbContext db, TeamEntity team, AccountEntity caller) =>
        new TeamsController().Delete(team.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            mutateTeams: new Mutate<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db));

    private static Task<ActionResult<TeamPresentation>> Get(PrimaryDbContext db, TeamEntity team,
        AccountEntity caller) =>
        new TeamsController().Get(team.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db),
            readProjects: new Read<ProjectEntity>(db));

    [Fact]
    public async Task TestCreate_ManagerJoinsOwnTeam()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var manager = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);

        var byManager = (await Create(db, organization, manager, "Backend")).Value!;
        var byOwner = (await Create(db, organization, ann, "Frontend")).Value!;

        Assert.Equal(["bob@test.com"], byManager.Members.Select(m => m.Email));
        Assert.True(byManager.CanManage);
        Assert.Empty(byOwner.Members);
        Assert.True(byOwner.CanManage);
    }

    [Fact]
    public async Task TestCreate_Rules()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var member = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);
        await OrgSupport.CreateTeam(db, organization, "Backend");

        ControllerAssert.HasValidationError((await Create(db, organization, member, "Ops")).Result,
            ErrorCode.Forbidden);
        ControllerAssert.HasValidationError((await Create(db, organization, ann, " Backend ")).Result,
            ErrorCode.AlreadyExist, "name");
        ControllerAssert.HasValidationError((await Create(db, organization, ann, " ")).Result,
            ErrorCode.ValueRequired, "name");
    }

    [Fact]
    public async Task TestManagerManagesOnlyOwnTeams()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var manager = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);
        var cid = await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);
        var own = await OrgSupport.CreateTeam(db, organization, "Backend", manager);
        var other = await OrgSupport.CreateTeam(db, organization, "Frontend");

        ControllerAssert.IsOkResult(await AddMember(db, own, manager, cid));
        ControllerAssert.HasValidationError(await AddMember(db, own, manager, cid), ErrorCode.AlreadyExist);
        ControllerAssert.IsOkResult(await RemoveMember(db, own, manager, cid));
        ControllerAssert.HasValidationError(await AddMember(db, other, manager, cid), ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await Delete(db, other, manager), ErrorCode.Forbidden);
        Assert.False((await Get(db, other, manager)).Value!.CanManage);

        ControllerAssert.IsOkResult(await Delete(db, own, manager));
        Assert.DoesNotContain(db.Teams, t => t.Id == own.Id);
    }

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

    [Fact]
    public async Task TestGet_TeamOfOtherOrganizationDoesntExist()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var team = await OrgSupport.CreateTeam(db, organization, "Backend");
        var stranger = await AccountSupport.CreateAccount(db, "bob@test.com");

        ControllerAssert.HasValidationError((await Get(db, team, stranger)).Result, ErrorCode.DoesntExist,
            "teamId");
    }

    [Fact]
    public async Task TestGetTeams_ListsCountsAndMembership()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var member = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);
        var backend = await OrgSupport.CreateTeam(db, organization, "Backend", ann, member);
        await OrgSupport.CreateTeam(db, organization, "Applied");
        await OrgSupport.CreateProject(db, organization, "Api", backend);

        var result = await new TeamsController().GetTeams(organization.Id, OrgSupport.Claims(member),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readTeams: new Read<TeamEntity>(db),
            readTeamMembers: new Read<TeamMemberEntity>(db),
            readProjectTeams: new Read<ProjectTeamEntity>(db));

        var teams = result.Value!;
        Assert.Equal(["Applied", "Backend"], teams.Select(t => t.Name));
        Assert.Equal((2, 1, true, false),
            (teams[1].MemberCount, teams[1].ProjectCount, teams[1].IsMember, teams[1].CanManage));
    }
}
