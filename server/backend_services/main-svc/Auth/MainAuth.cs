using System.Diagnostics.CodeAnalysis;
using System.Security.Claims;
using System.Text.Encodings.Web;
using core.Auth;
using core.Libs.EntityAccess;
using core.Operations;
using core.Services;
using db.Entities;
using Microsoft.Extensions.Options;

namespace main_svc.Auth;

public static class MainAuth
{
    /// <summary>The web app.</summary>
    public static readonly SessionPolicy CookiePolicy = new(TimeSpan.FromHours(12), TimeSpan.FromDays(7));

    /// <summary>The simpled CLI: stays signed in while used at least monthly, re-login every 90 days.</summary>
    public static readonly SessionPolicy TokenPolicy = new(TimeSpan.FromDays(30), TimeSpan.FromDays(90));

    public static SessionPolicy PolicyFor(AuthType authType) =>
        authType == AuthType.TokenSession ? TokenPolicy : CookiePolicy;
}

[ExcludeFromCodeCoverage] // Covered by the e2e suite.
public class MainAuthenticationHandler(
    IOptionsMonitor<SessionAuthenticationOptions> options,
    ILoggerFactory logger,
    UrlEncoder encoder,
    SessionCookie sessionCookie,
    Mutate<AuthSessionEntity> mutateSessions,
    UnitOfWork unitOfWork,
    Clock clock,
    Read<AccountEntity> readAccounts
) : SessionAuthenticationHandler(options, logger, encoder, sessionCookie, mutateSessions, unitOfWork, clock)
{
    protected override async Task<ClaimsPrincipal?> CreatePrincipal(AuthSessionEntity session, AuthType authType)
    {
        var account = await AccountOps.TryGetAccountById(session.AccountId, readAccounts);
        if (account == null || !account.Confirmed)
        {
            return null;
        }

        return ClaimsPrincipalExt.CreateClaimsPrincipal(new MainClaims
        {
            AccountId = session.AccountId,
            SessionId = session.Id,
            AuthType = authType,
        });
    }
}
