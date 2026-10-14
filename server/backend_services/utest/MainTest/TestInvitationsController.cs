using core.Auth;
using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Presentation;
using core.Primitive;
using db.Entities;
using main_svc.Controllers;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using primary_db;
using utest.TestSupport;
using utest.TestSupport.TestDoubles;

namespace utest.MainTest;

public class TestInvitationsController
{
    private static InvitationsController NewController()
    {
        var controller = new InvitationsController();
        controller.ControllerContext.HttpContext = new DefaultHttpContext();
        return controller;
    }

    private static Task<ActionResult<InvitationPresentation>> Invite(PrimaryDbContext db, RecordingMailer mailer,
        OrganizationEntity organization, AccountEntity caller, string email, string role) =>
        NewController().Invite(organization.Id, new InviteRequest { Email = email, Role = role },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateInvitations: new Mutate<OrgInvitationEntity>(db),
            userMailer: mailer.UserMailer());

    private static Task<ActionResult<InvitationPreviewPresentation>> Preview(PrimaryDbContext db, string token) =>
        NewController().Preview(new TokenRequest { Token = token },
            context: CallContextSupport.Main(db),
            readInvitations: new Read<OrgInvitationEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db));

    private static Task<ActionResult<MyOrganizationPresentation>> Accept(PrimaryDbContext db, AccountEntity caller,
        string token) =>
        NewController().Accept(new TokenRequest { Token = token },
            claims: OrgSupport.Claims(caller),
            context: CallContextSupport.Main(db),
            readInvitations: new Read<OrgInvitationEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db));

    private static Task<ActionResult<main_svc.Controllers.SessionTokenResponse>> Register(PrimaryDbContext db,
        string token, string password = AccountSupport.Password) =>
        NewController().Register(new InvitationRegisterRequest { Token = token, Name = "Bob", Password = password },
            useToken: true,
            context: CallContextSupport.Main(db),
            sessionCookie: new SessionCookie("session", SameSiteMode.Lax, Secure: false),
            readInvitations: new Read<OrgInvitationEntity>(db),
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db),
            mutateSessions: new Mutate<AuthSessionEntity>(db));

    [Fact]
    public async Task TestInvite_MailsLink()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);

        var result = await Invite(db, mailer, organization, ann, "Bob@Test.com", "manager");

        Assert.Equal(("bob@test.com", "manager", "Ann"), (result.Value!.Email, result.Value.Role,
            result.Value.InvitedBy));
        var (_, message) = Assert.Single(mailer.Sent);
        Assert.Equal("org_invitation", message.TemplateName);
        Assert.Equal("Acme", message.TemplateData["organization"]);
        Assert.StartsWith("http://app.test/invitation?token=", (string)message.TemplateData["link"]);
    }

    [Fact]
    public async Task TestInvite_RoleRules()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var admin = await OrgSupport.AddMember(db, organization, "adm@test.com", OrgRole.Admin);
        var manager = await OrgSupport.AddMember(db, organization, "mgr@test.com", OrgRole.Manager);

        ControllerAssert.HasValidationError((await Invite(db, mailer, organization, manager, "x@test.com",
            "member")).Result, ErrorCode.Forbidden);
        ControllerAssert.HasValidationError((await Invite(db, mailer, organization, admin, "x@test.com",
            "owner")).Result, ErrorCode.Forbidden);
        ControllerAssert.HasValidationError((await Invite(db, mailer, organization, admin, "mgr@test.com",
            "member")).Result, ErrorCode.AlreadyExist, "email");
        ControllerAssert.HasValidationError((await Invite(db, mailer, organization, admin, "not-an-email",
            "member")).Result, ErrorCode.IncorrectValue, "email");
        Assert.Empty(mailer.Sent);

        ControllerAssert.IsOkResult(await Invite(db, mailer, organization, admin, "x@test.com", "admin"));
    }

    [Fact]
    public async Task TestInvite_AgainReplacesEarlierLink()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);

        await Invite(db, mailer, organization, ann, "bob@test.com", "member");
        var first = mailer.LastToken("bob@test.com", "org_invitation");
        await Invite(db, mailer, organization, ann, "bob@test.com", "admin");
        var second = mailer.LastToken("bob@test.com", "org_invitation");

        ControllerAssert.HasValidationError((await Preview(db, first)).Result, ErrorCode.IncorrectValue, "token");
        Assert.Equal("admin", (await Preview(db, second)).Value!.Role);
    }

    [Fact]
    public async Task TestPreview_TellsWhetherAccountExists()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        await AccountSupport.CreateAccount(db, "bob@test.com");

        await Invite(db, mailer, organization, ann, "bob@test.com", "member");
        await Invite(db, mailer, organization, ann, "cid@test.com", "member");

        var bob = (await Preview(db, mailer.LastToken("bob@test.com", "org_invitation"))).Value!;
        var cid = (await Preview(db, mailer.LastToken("cid@test.com", "org_invitation"))).Value!;
        Assert.Equal(("Acme", "Ann", true), (bob.OrganizationName, bob.InvitedBy, bob.AccountExists));
        Assert.False(cid.AccountExists);
    }

    [Fact]
    public async Task TestAccept_JoinsWithInvitedRoleOnce()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var bob = await AccountSupport.CreateAccount(db, "bob@test.com");
        await Invite(db, mailer, organization, ann, "bob@test.com", "manager");
        var token = mailer.LastToken("bob@test.com", "org_invitation");

        var result = await Accept(db, bob, token);

        Assert.Equal(("Acme", "manager"), (result.Value!.Name, result.Value.Role));
        Assert.Equal("manager", OrgSupport.RoleOf(db, organization, bob));
        ControllerAssert.HasValidationError((await Accept(db, bob, token)).Result, ErrorCode.IncorrectValue,
            "token");
    }

    [Fact]
    public async Task TestAccept_OtherEmailRejected()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var cid = await AccountSupport.CreateAccount(db, "cid@test.com");
        await Invite(db, mailer, organization, ann, "bob@test.com", "member");

        var result = await Accept(db, cid, mailer.LastToken("bob@test.com", "org_invitation"));

        ControllerAssert.HasValidationError(result.Result, ErrorCode.IncorrectState, "token");
        Assert.DoesNotContain(db.OrgMembers, m => m.AccountId == cid.Id);
    }

    [Fact]
    public async Task TestRegister_CreatesConfirmedMemberAndSignsIn()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        await Invite(db, mailer, organization, ann, "bob@test.com", "admin");
        var token = mailer.LastToken("bob@test.com", "org_invitation");

        ControllerAssert.HasValidationError((await Register(db, token, "short")).Result, ErrorCode.IncorrectValue,
            "password");

        var result = await Register(db, token);

        Assert.NotEmpty(result.Value!.Token);
        var bob = db.Accounts.Single(a => a.Login == "bob@test.com");
        Assert.True(bob.Confirmed);
        Assert.Equal("admin", OrgSupport.RoleOf(db, organization, bob));
        Assert.Single(db.AuthSessions, s => s.AccountId == bob.Id);
        ControllerAssert.HasValidationError((await Register(db, token)).Result, ErrorCode.IncorrectValue, "token");
    }

    [Fact]
    public async Task TestRegister_ExistingAccountMustSignIn()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        await AccountSupport.CreateAccount(db, "bob@test.com");
        await Invite(db, mailer, organization, ann, "bob@test.com", "member");

        var result = await Register(db, mailer.LastToken("bob@test.com", "org_invitation"));

        ControllerAssert.HasValidationError(result.Result, ErrorCode.AlreadyExist, "email");
    }

    [Fact]
    public async Task TestRevoke_LinkStopsWorking()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        var (organization, ann) = await OrgSupport.CreateOrg(db);
        var invitation = (await Invite(db, mailer, organization, ann, "bob@test.com", "member")).Value!;

        var result = await NewController().Revoke(organization.Id, invitation.Id,
            claims: OrgSupport.Claims(ann),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readInvitations: new Read<OrgInvitationEntity>(db));

        ControllerAssert.IsOkResult(result);
        ControllerAssert.HasValidationError(
            (await Preview(db, mailer.LastToken("bob@test.com", "org_invitation"))).Result,
            ErrorCode.IncorrectValue, "token");
        var open = await NewController().GetInvitations(organization.Id,
            claims: OrgSupport.Claims(ann),
            context: CallContextSupport.Main(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db),
            readAccounts: new Read<AccountEntity>(db),
            readInvitations: new Read<OrgInvitationEntity>(db));
        Assert.Empty(open.Value!);
    }
}
