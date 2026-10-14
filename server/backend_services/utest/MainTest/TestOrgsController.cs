using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Primitive;
using db.Entities;
using main_svc.Controllers;
using Microsoft.AspNetCore.Mvc;
using primary_db;
using utest.TestSupport;

namespace utest.MainTest;

public class TestOrgsController
{
    private static Task<ActionResult> SetRole(PrimaryDbContext db, OrganizationEntity organization,
        AccountEntity caller, AccountEntity target, string role) =>
        new OrgsController().SetRole(organization.Id, target.ExternalId, new RoleRequest { Role = role },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db));

    private static Task<ActionResult> RemoveMember(PrimaryDbContext db, OrganizationEntity organization,
        AccountEntity caller, AccountEntity target) =>
        new OrgsController().RemoveMember(organization.Id, target.ExternalId,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateTeamMembers: new Mutate<TeamMemberEntity>(db),
            readTeams: new Read<TeamEntity>(db));

    [Fact]
    public async Task TestCreate_CallerOwnsNewOrganization()
    {
        var db = InMemoryPrimaryDb.Create();
        var (_, ann) = await OrgSupport.CreateOrg(db, "Acme");

        var result = await new OrgsController().Create(new OrganizationRequest { Name = "Acme" },
            claims: OrgSupport.Claims(ann),
            context: CallContextSupport.Main(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateOrganizations: new Mutate<OrganizationEntity>(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db));

        var organization = result.Value!;
        Assert.Equal("acme-2", organization.Slug);
        Assert.Equal("owner", organization.Role);
        Assert.True(organization.Permissions.CanManageOrganization);
        Assert.Equal(2, db.OrgMembers.Count(m => m.AccountId == ann.Id && m.Role == "owner"));
    }

    [Fact]
    public async Task TestGet_OtherOrganizationDoesntExist()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var stranger = await AccountSupport.CreateAccount(db, "bob@test.com");

        var result = await new OrgsController().Get(organization.Id, OrgSupport.Claims(stranger),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db));

        ControllerAssert.HasValidationError(result.Result, ErrorCode.DoesntExist, "orgId");
    }

    [Fact]
    public async Task TestGet_MemberSeesRoleWithoutPermissions()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var bob = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);

        var result = await new OrgsController().Get(organization.Id, OrgSupport.Claims(bob),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db));

        var presented = result.Value!;
        Assert.Equal("member", presented.Role);
        Assert.False(presented.Permissions.CanManageMembers);
        Assert.False(presented.Permissions.CanCreateTeam);
        Assert.Empty(presented.Permissions.AssignableRoles);
    }

    [Fact]
    public async Task TestUpdate_OnlyOwnerRenames()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Admin);

        Task<ActionResult<core.Presentation.OrganizationPresentation>> Rename(AccountEntity caller) =>
            new OrgsController().Update(organization.Id, new OrganizationRequest { Name = " Acme Inc " },
                claims: OrgSupport.Claims(caller),
                context: CallContextSupport.Main(db),
                readMembers: new Read<OrgMemberEntity>(db),
                readOrganizations: new Read<OrganizationEntity>(db));

        ControllerAssert.HasValidationError((await Rename(admin)).Result, ErrorCode.Forbidden);

        var result = await Rename(ann);
        Assert.Equal("Acme Inc", result.Value!.Name);
        Assert.Equal("acme", result.Value.Slug);
    }

    [Fact]
    public async Task TestDelete_OwnerDeletes()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Admin);

        Task<ActionResult> Delete(AccountEntity caller) => new OrgsController().Delete(organization.Id,
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            mutateOrganizations: new Mutate<OrganizationEntity>(db));

        ControllerAssert.HasValidationError(await Delete(admin), ErrorCode.Forbidden);
        ControllerAssert.IsOkResult(await Delete(ann));
        Assert.Empty(db.Organizations);
    }

    [Fact]
    public async Task TestGetMembers_FlagsWhatCallerMayDo()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Admin);
        await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);

        var result = await new OrgsController().GetMembers(organization.Id, OrgSupport.Claims(admin),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db));

        var members = result.Value!.ToDictionary(m => m.Email);
        Assert.False(members["ann@test.com"].CanChangeRole);
        Assert.False(members["ann@test.com"].CanRemove);
        Assert.True(members["cid@test.com"].CanChangeRole);
        Assert.True(members["bob@test.com"].IsMe);
        Assert.True(members["bob@test.com"].CanRemove);
    }

    [Fact]
    public async Task TestSetRole_AdminManagesNonOwners()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Admin);
        var cid = await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);

        ControllerAssert.IsOkResult(await SetRole(db, organization, admin, cid, "manager"));
        Assert.Equal("manager", OrgSupport.RoleOf(db, organization, cid));

        ControllerAssert.HasValidationError(await SetRole(db, organization, admin, cid, "owner"),
            ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await SetRole(db, organization, admin, ann, "member"),
            ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await SetRole(db, organization, admin, cid, "boss"),
            ErrorCode.IncorrectValue, "role");
    }

    [Fact]
    public async Task TestSetRole_ManagerAndMemberForbidden()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var manager = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);
        var cid = await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);

        ControllerAssert.HasValidationError(await SetRole(db, organization, manager, cid, "member"),
            ErrorCode.Forbidden);
        ControllerAssert.HasValidationError(await SetRole(db, organization, cid, cid, "admin"),
            ErrorCode.Forbidden);
    }

    [Fact]
    public async Task TestSetRole_LastOwnerStays()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var bob = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);

        ControllerAssert.HasValidationError(await SetRole(db, organization, ann, ann, "admin"),
            ErrorCode.IncorrectState, "accountId");

        ControllerAssert.IsOkResult(await SetRole(db, organization, ann, bob, "owner"));
        ControllerAssert.IsOkResult(await SetRole(db, organization, ann, ann, "admin"));
        Assert.Equal("admin", OrgSupport.RoleOf(db, organization, ann));
    }

    [Fact]
    public async Task TestSetRole_UnknownAccountDoesntExist()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var stranger = await AccountSupport.CreateAccount(db, "bob@test.com");

        ControllerAssert.HasValidationError(await SetRole(db, organization, ann, stranger, "member"),
            ErrorCode.DoesntExist, "accountId");
    }

    [Fact]
    public async Task TestRemoveMember_DropsTeamMemberships()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var bob = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Manager);
        await OrgSupport.CreateTeam(db, organization, "Core", ann, bob);

        ControllerAssert.IsOkResult(await RemoveMember(db, organization, ann, bob));

        Assert.DoesNotContain(db.OrgMembers, m => m.AccountId == bob.Id);
        Assert.DoesNotContain(db.TeamMembers, m => m.AccountId == bob.Id);
        Assert.Contains(db.TeamMembers, m => m.AccountId == ann.Id);
    }

    [Fact]
    public async Task TestRemoveMember_AnyoneLeavesButLastOwner()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var bob = await OrgSupport.AddMember(db, organization, "bob@test.com", OrgRole.Member);
        var cid = await OrgSupport.AddMember(db, organization, "cid@test.com", OrgRole.Member);

        ControllerAssert.HasValidationError(await RemoveMember(db, organization, bob, cid), ErrorCode.Forbidden);
        ControllerAssert.IsOkResult(await RemoveMember(db, organization, bob, bob));
        ControllerAssert.HasValidationError(await RemoveMember(db, organization, ann, ann),
            ErrorCode.IncorrectState, "accountId");
    }
}
