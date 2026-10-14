using System.Diagnostics.CodeAnalysis;
using System.Security.Claims;
using System.Text.Encodings.Web;
using core.Libs.EntityAccess;
using core.Operations;
using core.Services;
using db.Entities;
using Microsoft.AspNetCore.Authentication;
using Microsoft.Extensions.Logging;
using Microsoft.Extensions.Options;

namespace core.Auth;

public class SessionAuthenticationOptions : AuthenticationSchemeOptions
{
    /// <summary><see cref="AuthSessionEntity"/> audience this service accepts.</summary>
    public string Audience { get; set; } = "";

    /// <summary>Whether an <c>Authorization: Bearer</c> header is accepted next to the cookie.</summary>
    public bool AcceptBearer { get; set; }
}

/// <summary>
/// Resolves the bearer token or session cookie to a live <see cref="AuthSessionEntity"/> and asks the service
/// for the principal, so roles and ids are read fresh on every request instead of frozen at login.
/// </summary>
[ExcludeFromCodeCoverage] // Covered by the e2e suite; the session logic itself is in AuthSessionOps.
public abstract class SessionAuthenticationHandler(
    IOptionsMonitor<SessionAuthenticationOptions> options,
    ILoggerFactory logger,
    UrlEncoder encoder,
    SessionCookie sessionCookie,
    Mutate<AuthSessionEntity> mutateSessions,
    UnitOfWork unitOfWork,
    Clock clock
) : AuthenticationHandler<SessionAuthenticationOptions>(options, logger, encoder)
{
    public const string SchemeName = "Session";

    private const string BearerPrefix = "Bearer ";

    /// <summary>The service's principal for <paramref name="session"/>, or null when the account may no longer log in.</summary>
    protected abstract Task<ClaimsPrincipal?> CreatePrincipal(AuthSessionEntity session, AuthType authType);

    protected override async Task<AuthenticateResult> HandleAuthenticateAsync()
    {
        var (token, authType) = ReadToken();
        if (token == null)
        {
            return AuthenticateResult.NoResult();
        }

        var now = clock.Now;
        var session = await AuthSessionOps.TryGetActiveSession(token, Options.Audience, authType, now, mutateSessions);
        if (session == null)
        {
            return AuthenticateResult.Fail("Session is missing, expired or revoked");
        }

        var principal = await CreatePrincipal(session, authType);
        if (principal == null)
        {
            return AuthenticateResult.Fail("Account may no longer log in");
        }

        if (AuthSessionOps.Touch(session, now))
        {
            await unitOfWork.SaveChangesAsync(Context.RequestAborted);
        }

        return AuthenticateResult.Success(new AuthenticationTicket(principal, Scheme.Name));
    }

    private (string? Token, AuthType AuthType) ReadToken()
    {
        string authorization = Request.Headers.Authorization.ToString();
        if (Options.AcceptBearer && authorization.StartsWith(BearerPrefix, StringComparison.Ordinal))
        {
            var token = authorization[BearerPrefix.Length..].Trim();
            return (token.Length > 0 ? token : null, AuthType.TokenSession);
        }

        return (sessionCookie.Read(Request), AuthType.CookieSession);
    }
}
