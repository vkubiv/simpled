using core.Auth;
using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using primary_db;
using utest.TestSupport;

namespace utest.Core.Operations;

public class TestAuthSessionOps
{
    private static readonly DateTime T0 = new(2026, 9, 1, 12, 0, 0, DateTimeKind.Utc);
    private static readonly SessionPolicy Policy = new(TimeSpan.FromHours(1), TimeSpan.FromDays(1));

    private static async Task<string> Create(PrimaryDbContext db, int accountId = 7,
        string audience = AuthSessionEntity.AudienceMain, AuthType authType = AuthType.TokenSession)
    {
        var (token, _) = await AuthSessionOps.CreateSession(
            accountId: accountId,
            audience: audience,
            authType: authType,
            policy: Policy,
            userAgent: new string('x', 1000),
            now: T0,
            mutateSessions: new Mutate<AuthSessionEntity>(db));
        await db.SaveChangesAsync();
        return token;
    }

    private static Task<AuthSessionEntity?> Find(PrimaryDbContext db, string token, DateTime now,
        string audience = AuthSessionEntity.AudienceMain, AuthType authType = AuthType.TokenSession) =>
        AuthSessionOps.TryGetActiveSession(token, audience, authType, now, new Mutate<AuthSessionEntity>(db));

    [Fact]
    public async Task TestCreateSession_StoresOnlyTheHash()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);

        var session = Assert.Single(db.AuthSessions);
        Assert.NotEqual(token, session.TokenHash);
        Assert.Equal(AuthSessionOps.HashToken(token), session.TokenHash);
        Assert.Equal(T0 + Policy.AbsoluteLifetime, session.ExpiresAt);
        Assert.Equal(256, session.UserAgent!.Length);
        Assert.NotEqual(token, await Create(db));
    }

    [Fact]
    public async Task TestTryGetActiveSession_Found()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);

        var session = await Find(db, token, T0.AddMinutes(5));
        Assert.Equal(7, session?.AccountId);
    }

    [Fact]
    public async Task TestTryGetActiveSession_UnknownToken()
    {
        var db = InMemoryPrimaryDb.Create();
        await Create(db);

        Assert.Null(await Find(db, "not-a-token", T0));
    }

    [Fact]
    public async Task TestTryGetActiveSession_OtherAudienceOrTransport()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);

        Assert.Null(await Find(db, token, T0, audience: "other"));
        Assert.Null(await Find(db, token, T0, authType: AuthType.CookieSession));
    }

    [Fact]
    public async Task TestTryGetActiveSession_IdleTimeout()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);

        Assert.Null(await Find(db, token, T0 + Policy.IdleTimeout));
    }

    [Fact]
    public async Task TestTryGetActiveSession_TouchExtendsIdleButNotLifetime()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);

        var now = T0;
        while (now + TimeSpan.FromMinutes(50) < T0 + Policy.AbsoluteLifetime)
        {
            now += TimeSpan.FromMinutes(50);
            var session = await Find(db, token, now);
            Assert.NotNull(session);
            Assert.True(AuthSessionOps.Touch(session, now));
            await db.SaveChangesAsync();
        }

        Assert.Null(await Find(db, token, T0 + Policy.AbsoluteLifetime));
    }

    [Fact]
    public async Task TestTouch_ThrottledWithinInterval()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);
        var session = (await Find(db, token, T0))!;

        Assert.False(AuthSessionOps.Touch(session, T0 + AuthSessionOps.TouchInterval / 2));
        Assert.Equal(T0, session.LastSeenAt);
    }

    [Fact]
    public async Task TestRevokeSession()
    {
        var db = InMemoryPrimaryDb.Create();
        var token = await Create(db);
        var other = await Create(db);
        var session = (await Find(db, token, T0))!;

        await AuthSessionOps.RevokeSession(session.Id, T0, new Mutate<AuthSessionEntity>(db));
        await db.SaveChangesAsync();

        Assert.Null(await Find(db, token, T0));
        Assert.NotNull(await Find(db, other, T0));
    }

    [Fact]
    public async Task TestRevokeAllSessions_OnlyThatAccount()
    {
        var db = InMemoryPrimaryDb.Create();
        var tokenSession = await Create(db, accountId: 7);
        var cookieSession = await Create(db, accountId: 7, authType: AuthType.CookieSession);
        var otherAccount = await Create(db, accountId: 8);

        await AuthSessionOps.RevokeAllSessions(7, T0, new Mutate<AuthSessionEntity>(db));
        await db.SaveChangesAsync();

        Assert.Null(await Find(db, tokenSession, T0));
        Assert.Null(await Find(db, cookieSession, T0, authType: AuthType.CookieSession));
        Assert.NotNull(await Find(db, otherAccount, T0));
    }
}
