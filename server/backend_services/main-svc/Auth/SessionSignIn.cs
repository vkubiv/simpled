using core.Auth;
using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using main_svc.Controllers;
using Microsoft.AspNetCore.Mvc;

namespace main_svc.Auth;

public static class SessionSignIn
{
    /// <summary>
    /// Opens a session and saves the unit of work. <c>?useToken=true</c> (the CLI) gets the token in the body;
    /// the web app gets an HttpOnly cookie and an empty 200.
    /// </summary>
    public static async Task<ActionResult<SessionTokenResponse>> SignIn(
        ControllerBase controller,
        AccountEntity account,
        bool useToken,
        MainCallContext context,
        SessionCookie sessionCookie,
        Mutate<AuthSessionEntity> mutateSessions)
    {
        var authType = useToken ? AuthType.TokenSession : AuthType.CookieSession;
        var (token, _) = await AuthSessionOps.CreateSession(
            accountId: account.Id,
            audience: AuthSessionEntity.AudienceMain,
            authType: authType,
            policy: MainAuth.PolicyFor(authType),
            userAgent: controller.Request.Headers.UserAgent.ToString(),
            now: context.Clock.Now,
            mutateSessions: mutateSessions);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        if (useToken)
        {
            return new SessionTokenResponse { Token = token };
        }

        sessionCookie.Write(controller.Response, token);
        return controller.Ok();
    }
}
