using core.Auth;
using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Primitive;
using core.Services.Mailer;
using core.Validations;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.RateLimiting;

namespace main_svc.Controllers;

public class LoginRequest
{
    public required string Email { get; set; }
    public required string Password { get; set; }
}

/// <summary>Only for <c>?useToken=true</c> (the CLI); the web app gets an HttpOnly cookie and an empty 200.</summary>
public class SessionTokenResponse
{
    public required string Token { get; set; }
}

public class EmailNeedsConfirmationResponse
{
    public required bool EmailNeedsConfirmation { get; set; }
}

public class TokenRequest
{
    public required string Token { get; set; }
}

public class EmailRequest
{
    public required string Email { get; set; }
}

public class ResetPasswordRequest
{
    public required string Token { get; set; }
    public required string NewPassword { get; set; }
}

[EnableRateLimiting(AuthRateLimiting.Policy)]
[ApiController]
[Route("[controller]")]
public class AuthController : ControllerBase
{
    /// <summary>Creates the account and its first organization, owned by it, and mails a confirmation link.</summary>
    [AllowAnonymous]
    [HttpPost("Register")]
    public async Task<ActionResult<EmailNeedsConfirmationResponse>> Register(
        Registration request,
        [FromServices] MainCallContext context,
        [FromServices] Mutate<AccountEntity> mutateAccounts,
        [FromServices] Mutate<OrganizationEntity> mutateOrganizations,
        [FromServices] Mutate<OrgMemberEntity> mutateMembers,
        [FromServices] Mutate<ConfirmationTokenEntity> mutateTokens,
        [FromServices] UserMailer userMailer
    )
    {
        using var vr = new ValidationResult();
        if (!AccountValidation.ValidateRegistration(request, vr) ||
            !await AccountValidation.EnsureAccountNotExist(request.Email, vr, mutateAccounts.Read))
        {
            return vr.ReturnErrorResponse();
        }

        var now = context.Clock.Now;
        var account = await AccountOps.CreateAccount(
            login: request.Email,
            name: request.Name,
            password: request.Password,
            confirmed: false,
            now: now,
            mutateAccounts: mutateAccounts);

        await OrganizationOps.Create(
            name: request.OrganizationName,
            owner: account,
            now: now,
            mutateOrganizations: mutateOrganizations,
            mutateMembers: mutateMembers);

        var token = await ConfirmationTokenOps.Create(
            account: account,
            purpose: ConfirmationTokenEntity.PurposeEmailConfirmation,
            now: now,
            mutateTokens: mutateTokens);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        await userMailer.SendEmailConfirmation(account.Login, account.Name, token);

        return new EmailNeedsConfirmationResponse { EmailNeedsConfirmation = true };
    }

    /// <summary>Confirms the email behind the mailed token and signs in.</summary>
    [AllowAnonymous]
    [HttpPost("ConfirmEmail")]
    public async Task<ActionResult<SessionTokenResponse>> ConfirmEmail(
        TokenRequest request,
        [FromQuery(Name = "useToken")] bool useToken,
        [FromServices] MainCallContext context,
        [FromServices] SessionCookie sessionCookie,
        [FromServices] Mutate<AccountEntity> mutateAccounts,
        [FromServices] Mutate<ConfirmationTokenEntity> mutateTokens,
        [FromServices] Mutate<AuthSessionEntity> mutateSessions
    )
    {
        var now = context.Clock.Now;

        using var vr = new ValidationResult();
        var token = await ConfirmationTokenOps.TryRedeem(request.Token,
            ConfirmationTokenEntity.PurposeEmailConfirmation, now, mutateTokens);
        if (token == null)
        {
            vr.IncorrectValue("token", "The link is invalid or has expired.");
            return vr.ReturnErrorResponse();
        }

        var account = (await AccountOps.TryGetAccountById(token.AccountId, mutateAccounts.Read))!;
        account.Confirmed = true;

        return await SessionSignIn.SignIn(this, account, useToken, context, sessionCookie, mutateSessions);
    }

    /// <summary>Always 200, so the endpoint does not reveal which emails have accounts.</summary>
    [AllowAnonymous]
    [HttpPost("ResendConfirmation")]
    public async Task<ActionResult> ResendConfirmation(
        EmailRequest request,
        [FromServices] MainCallContext context,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<ConfirmationTokenEntity> mutateTokens,
        [FromServices] UserMailer userMailer
    )
    {
        var account = await AccountOps.TryGetAccountByLogin(request.Email, readAccounts);
        if (account == null || account.Confirmed)
        {
            return Ok();
        }

        var token = await ConfirmationTokenOps.Create(
            account: account,
            purpose: ConfirmationTokenEntity.PurposeEmailConfirmation,
            now: context.Clock.Now,
            mutateTokens: mutateTokens);
        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        await userMailer.SendEmailConfirmation(account.Login, account.Name, token);
        return Ok();
    }

    [AllowAnonymous]
    [HttpPost("Login")]
    public async Task<ActionResult<SessionTokenResponse>> Login(
        LoginRequest request,
        [FromQuery(Name = "useToken")] bool useToken,
        [FromServices] MainCallContext context,
        [FromServices] SessionCookie sessionCookie,
        [FromServices] Mutate<AccountEntity> mutateAccounts,
        [FromServices] Mutate<AuthSessionEntity> mutateSessions
    )
    {
        var account = await AccountOps.TryLogin(
            new Credentials { Login = request.Email, Password = request.Password },
            now: context.Clock.Now,
            mutateAccounts: mutateAccounts);
        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        if (account == null)
        {
            return Unauthorized();
        }

        using var vr = new ValidationResult();
        if (!account.Confirmed)
        {
            vr.ValidationError("email-not-confirmed", "email", "The email address is not confirmed yet.");
            return vr.ReturnErrorResponse();
        }

        return await SessionSignIn.SignIn(this, account, useToken, context, sessionCookie, mutateSessions);
    }

    [HttpPost("Logout")]
    public async Task<ActionResult> Logout(
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] SessionCookie sessionCookie,
        [FromServices] Mutate<AuthSessionEntity> mutateSessions
    )
    {
        await AuthSessionOps.RevokeSession(claims.SessionId, context.Clock.Now, mutateSessions);
        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        sessionCookie.Delete(Response);
        return Ok();
    }

    /// <summary>Always 200, so the endpoint does not reveal which emails have accounts.</summary>
    [AllowAnonymous]
    [HttpPost("RequestPasswordReset")]
    public async Task<ActionResult> RequestPasswordReset(
        EmailRequest request,
        [FromServices] MainCallContext context,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<ConfirmationTokenEntity> mutateTokens,
        [FromServices] UserMailer userMailer
    )
    {
        var account = await AccountOps.TryGetAccountByLogin(request.Email, readAccounts);
        if (account == null)
        {
            return Ok();
        }

        var token = await ConfirmationTokenOps.Create(
            account: account,
            purpose: ConfirmationTokenEntity.PurposeResetPassword,
            now: context.Clock.Now,
            mutateTokens: mutateTokens);
        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        await userMailer.SendResetPassword(account.Login, account.Name, token);
        return Ok();
    }

    /// <summary>Sets the new password, confirms the email (the link proves it) and signs out every session.</summary>
    [AllowAnonymous]
    [HttpPost("ResetPassword")]
    public async Task<ActionResult> ResetPassword(
        ResetPasswordRequest request,
        [FromServices] MainCallContext context,
        [FromServices] Mutate<AccountEntity> mutateAccounts,
        [FromServices] Mutate<ConfirmationTokenEntity> mutateTokens,
        [FromServices] Mutate<AuthSessionEntity> mutateSessions
    )
    {
        var now = context.Clock.Now;

        using var vr = new ValidationResult();
        var token = await ConfirmationTokenOps.TryGetLive(request.Token,
            ConfirmationTokenEntity.PurposeResetPassword, now, mutateTokens);
        if (token == null)
        {
            vr.IncorrectValue("token", "The link is invalid or has expired.");
            return vr.ReturnErrorResponse();
        }

        // Validated before the token is used, so the person can retry the same link with a valid password.
        var account = (await AccountOps.TryGetAccountById(token.AccountId, mutateAccounts.Read))!;
        if (!AccountValidation.ValidatePassword(request.NewPassword, account.Login, vr))
        {
            return vr.ReturnErrorResponse();
        }

        token.UsedAt = now;
        AccountOps.SetPassword(account, request.NewPassword);
        account.Confirmed = true;
        await AuthSessionOps.RevokeAllSessions(account.Id, now, mutateSessions);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }
}
