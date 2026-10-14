using core.Auth;
using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Primitive;
using db.Entities;
using main_svc.Auth;
using main_svc.Controllers;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;
using primary_db;
using utest.TestSupport;
using utest.TestSupport.TestDoubles;

namespace utest.MainTest;

public class TestAuthController
{
    private static readonly SessionCookie Cookie = new("session", SameSiteMode.Lax, Secure: false);

    private static AuthController NewController()
    {
        var controller = new AuthController();
        controller.ControllerContext.HttpContext = new DefaultHttpContext();
        return controller;
    }

    private static Registration Registration(string email = "Ann@Test.com") => new()
    {
        Email = email,
        Password = AccountSupport.Password,
        Name = "Ann",
        OrganizationName = "Acme Corp",
    };

    private static Task<ActionResult<EmailNeedsConfirmationResponse>> Register(PrimaryDbContext db,
        RecordingMailer mailer, Registration registration) =>
        NewController().Register(registration,
            context: CallContextSupport.Main(db),
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateOrganizations: new Mutate<OrganizationEntity>(db),
            mutateMembers: new Mutate<OrgMemberEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            userMailer: mailer.UserMailer());

    private static Task<ActionResult<SessionTokenResponse>> ConfirmEmail(PrimaryDbContext db, string token,
        bool useToken = false) =>
        NewController().ConfirmEmail(new TokenRequest { Token = token }, useToken,
            context: CallContextSupport.Main(db),
            sessionCookie: Cookie,
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            mutateSessions: new Mutate<AuthSessionEntity>(db));

    private static Task<ActionResult<SessionTokenResponse>> Login(PrimaryDbContext db, string email,
        string password, bool useToken = true) =>
        NewController().Login(new LoginRequest { Email = email, Password = password }, useToken,
            context: CallContextSupport.Main(db),
            sessionCookie: Cookie,
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateSessions: new Mutate<AuthSessionEntity>(db));

    [Fact]
    public async Task TestRegister_CreatesUnconfirmedOwnerAndMailsLink()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();

        var result = await Register(db, mailer, Registration());

        Assert.True(result.Value?.EmailNeedsConfirmation);
        var account = Assert.Single(db.Accounts);
        Assert.Equal("ann@test.com", account.Login);
        Assert.False(account.Confirmed);
        var organization = Assert.Single(db.Organizations);
        Assert.Equal("acme-corp", organization.Slug);
        var member = Assert.Single(db.OrgMembers);
        Assert.Equal((account.Id, organization.Id, "owner"), (member.AccountId, member.OrganizationId, member.Role));
        Assert.NotEmpty(mailer.LastToken("ann@test.com", "email_confirmation"));
    }

    [Fact]
    public async Task TestRegister_EmailTaken()
    {
        var db = InMemoryPrimaryDb.Create();
        await AccountSupport.CreateAccount(db, "ann@test.com");

        var result = await Register(db, new RecordingMailer(), Registration("ANN@test.com"));

        ControllerAssert.HasValidationError(result.Result, ErrorCode.AlreadyExist, "email");
    }

    [Fact]
    public async Task TestRegister_InvalidInput()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();

        var shortPassword = Registration();
        shortPassword.Password = "short";
        ControllerAssert.HasValidationError((await Register(db, mailer, shortPassword)).Result,
            ErrorCode.IncorrectValue, "password");

        var noOrganization = Registration();
        noOrganization.OrganizationName = " ";
        ControllerAssert.HasValidationError((await Register(db, mailer, noOrganization)).Result,
            ErrorCode.ValueRequired, "organizationName");

        Assert.Empty(db.Accounts);
        Assert.Empty(mailer.Sent);
    }

    [Fact]
    public async Task TestLogin_BlockedUntilConfirmed()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        await Register(db, mailer, Registration());

        var beforeConfirm = await Login(db, "ann@test.com", AccountSupport.Password);
        ControllerAssert.HasValidationError(beforeConfirm.Result, "email-not-confirmed", "email");

        var confirmed = await ConfirmEmail(db, mailer.LastToken("ann@test.com", "email_confirmation"),
            useToken: true);
        Assert.NotEmpty(confirmed.Value!.Token);
        Assert.True(Assert.Single(db.Accounts).Confirmed);

        var afterConfirm = await Login(db, "ANN@test.com", AccountSupport.Password);
        Assert.NotEmpty(afterConfirm.Value!.Token);
        Assert.Equal(2, db.AuthSessions.Count());
    }

    [Fact]
    public async Task TestConfirmEmail_TokenIsSingleUse()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        await Register(db, mailer, Registration());
        var token = mailer.LastToken("ann@test.com", "email_confirmation");

        Assert.NotNull((await ConfirmEmail(db, token)).Result as OkResult);
        ControllerAssert.HasValidationError((await ConfirmEmail(db, token)).Result, ErrorCode.IncorrectValue, "token");
        ControllerAssert.HasValidationError((await ConfirmEmail(db, "unknown")).Result, ErrorCode.IncorrectValue,
            "token");
    }

    [Fact]
    public async Task TestConfirmEmail_SetsSessionCookie()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        await Register(db, mailer, Registration());
        var controller = NewController();

        await controller.ConfirmEmail(
            new TokenRequest { Token = mailer.LastToken("ann@test.com", "email_confirmation") }, false,
            context: CallContextSupport.Main(db),
            sessionCookie: Cookie,
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            mutateSessions: new Mutate<AuthSessionEntity>(db));

        Assert.Contains("session=", controller.Response.Headers.SetCookie.ToString(), StringComparison.Ordinal);
        Assert.Equal(AuthSessionEntity.TransportCookie, Assert.Single(db.AuthSessions).Transport);
    }

    [Fact]
    public async Task TestResendConfirmation_OnlyLatestLinkWorks()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        await Register(db, mailer, Registration());
        var first = mailer.LastToken("ann@test.com", "email_confirmation");

        await NewController().ResendConfirmation(new EmailRequest { Email = "ann@test.com" },
            context: CallContextSupport.Main(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            userMailer: mailer.UserMailer());
        var second = mailer.LastToken("ann@test.com", "email_confirmation");

        ControllerAssert.HasValidationError((await ConfirmEmail(db, first)).Result, ErrorCode.IncorrectValue, "token");
        Assert.NotNull((await ConfirmEmail(db, second)).Result as OkResult);
    }

    [Fact]
    public async Task TestLogin_WrongPassword()
    {
        var db = InMemoryPrimaryDb.Create();
        await AccountSupport.CreateAccount(db);

        Assert.IsType<UnauthorizedResult>((await Login(db, "ann@test.com", "wrong-password")).Result);
        Assert.IsType<UnauthorizedResult>((await Login(db, "nobody@test.com", AccountSupport.Password)).Result);
        Assert.Equal(1, Assert.Single(db.Accounts).FailedLoginCount);
    }

    [Fact]
    public async Task TestResetPassword_ChangesPasswordAndRevokesSessions()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();
        await AccountSupport.CreateAccount(db);
        await Login(db, "ann@test.com", AccountSupport.Password);

        await NewController().RequestPasswordReset(new EmailRequest { Email = "ann@test.com" },
            context: CallContextSupport.Main(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            userMailer: mailer.UserMailer());
        var token = mailer.LastToken("ann@test.com", "reset_password");

        Task<ActionResult> Reset(string password) => NewController().ResetPassword(
            new ResetPasswordRequest { Token = token, NewPassword = password },
            context: CallContextSupport.Main(db),
            mutateAccounts: new Mutate<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            mutateSessions: new Mutate<AuthSessionEntity>(db));

        ControllerAssert.HasValidationError(await Reset("short"), ErrorCode.IncorrectValue, "password");
        ControllerAssert.IsOkResult(await Reset("a-brand-new-password"));

        Assert.NotNull(Assert.Single(db.AuthSessions).RevokedAt);
        Assert.IsType<UnauthorizedResult>((await Login(db, "ann@test.com", AccountSupport.Password)).Result);
        Assert.NotEmpty((await Login(db, "ann@test.com", "a-brand-new-password")).Value!.Token);
    }

    [Fact]
    public async Task TestRequestPasswordReset_UnknownEmailIsSilent()
    {
        var db = InMemoryPrimaryDb.Create();
        var mailer = new RecordingMailer();

        var result = await NewController().RequestPasswordReset(new EmailRequest { Email = "nobody@test.com" },
            context: CallContextSupport.Main(db),
            readAccounts: new Read<AccountEntity>(db),
            mutateTokens: new Mutate<ConfirmationTokenEntity>(db),
            userMailer: mailer.UserMailer());

        ControllerAssert.IsOkResult(result);
        Assert.Empty(mailer.Sent);
    }

    [Fact]
    public async Task TestLogout_RevokesSession()
    {
        var db = InMemoryPrimaryDb.Create();
        await AccountSupport.CreateAccount(db);
        await Login(db, "ann@test.com", AccountSupport.Password);
        var session = Assert.Single(db.AuthSessions);

        var result = await NewController().Logout(
            new MainClaims { AccountId = session.AccountId, SessionId = session.Id, AuthType = AuthType.TokenSession },
            context: CallContextSupport.Main(db),
            sessionCookie: Cookie,
            mutateSessions: new Mutate<AuthSessionEntity>(db));

        ControllerAssert.IsOkResult(result);
        Assert.NotNull(Assert.Single(db.AuthSessions).RevokedAt);
    }
}
