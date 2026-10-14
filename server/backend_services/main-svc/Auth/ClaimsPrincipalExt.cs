using System.Globalization;
using System.Security.Claims;
using core.Auth;

namespace main_svc.Auth;

public static class ClaimsPrincipalExt
{
    private const string TokenSession = "TokenSession";
    private const string CookieSession = "CookieSession";
    private const string AccountIdClaim = "AccountId";
    private const string SessionIdClaim = "SessionId";

    public static MainClaims ToAuthClaims(this ClaimsPrincipal claimsPrincipal)
    {
        AuthType? authType = claimsPrincipal.Identity?.AuthenticationType switch
        {
            TokenSession => AuthType.TokenSession,
            CookieSession => AuthType.CookieSession,
            _ => null,
        };

        if (authType == null ||
            !int.TryParse(claimsPrincipal.FindFirst(AccountIdClaim)?.Value, out var accountId) ||
            !long.TryParse(claimsPrincipal.FindFirst(SessionIdClaim)?.Value, out var sessionId))
        {
            throw new InvalidOperationException("Session in bad state");
        }

        return new MainClaims
        {
            AuthType = authType.Value,
            AccountId = accountId,
            SessionId = sessionId,
        };
    }

    public static ClaimsPrincipal CreateClaimsPrincipal(MainClaims authClaims)
    {
        var claims = new List<Claim>
        {
            new(AccountIdClaim, authClaims.AccountId.ToString(CultureInfo.InvariantCulture)),
            new(SessionIdClaim, authClaims.SessionId.ToString(CultureInfo.InvariantCulture)),
        };

        var authType = authClaims.AuthType == AuthType.TokenSession ? TokenSession : CookieSession;
        return new ClaimsPrincipal(new ClaimsIdentity(claims, authType));
    }
}
