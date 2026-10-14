using System.Security.Cryptography;
using System.Text;
using core.Auth;
using core.Libs.EntityAccess;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class AuthSessionOps
{
    /// <summary>last_seen_at is written at most this often, so a busy client is not a write per request.</summary>
    public static readonly TimeSpan TouchInterval = TimeSpan.FromMinutes(1);

    private const int MaxUserAgentLength = 256;

    /// <summary>Returns the raw token; only its hash is stored. Saved with the caller's unit of work.</summary>
    public static async Task<(string Token, AuthSessionEntity Session)> CreateSession(
        int accountId,
        string audience,
        AuthType authType,
        SessionPolicy policy,
        string? userAgent,
        DateTime now,
        Mutate<AuthSessionEntity> mutateSessions)
    {
        var token = NewToken();

        var session = new AuthSessionEntity
        {
            TokenHash = HashToken(token),
            AccountId = accountId,
            Audience = audience,
            Transport = ToTransport(authType),
            CreatedAt = now,
            LastSeenAt = now,
            IdleTimeout = policy.IdleTimeout,
            ExpiresAt = now + policy.AbsoluteLifetime,
            UserAgent = userAgent?.Length > MaxUserAgentLength ? userAgent[..MaxUserAgentLength] : userAgent,
        };

        await mutateSessions.AddAsync(session);
        return (token, session);
    }

    /// <summary>The live session behind <paramref name="token"/>, tracked so it can be touched or revoked.</summary>
    public static async Task<AuthSessionEntity?> TryGetActiveSession(
        string token,
        string audience,
        AuthType authType,
        DateTime now,
        Mutate<AuthSessionEntity> mutateSessions)
    {
        var tokenHash = HashToken(token);
        var session = await mutateSessions.Read.FirstOrDefaultAsync(s => s.TokenHash == tokenHash);

        return session != null && session.Audience == audience && session.Transport == ToTransport(authType) &&
               IsActive(session, now)
            ? session
            : null;
    }

    public static bool IsActive(AuthSessionEntity session, DateTime now) =>
        session.RevokedAt == null && now < session.ExpiresAt && now < session.LastSeenAt + session.IdleTimeout;

    /// <summary>True when last_seen_at moved and needs saving.</summary>
    public static bool Touch(AuthSessionEntity session, DateTime now)
    {
        if (now - session.LastSeenAt < TouchInterval)
        {
            return false;
        }

        session.LastSeenAt = now;
        return true;
    }

    public static async Task RevokeSession(long sessionId, DateTime now, Mutate<AuthSessionEntity> mutateSessions)
    {
        var session = await mutateSessions.Read.FirstOrDefaultAsync(s => s.Id == sessionId);
        if (session != null && session.RevokedAt == null)
        {
            session.RevokedAt = now;
        }
    }

    /// <summary>Every live session of the account: password changes, lockouts, removals.</summary>
    public static async Task RevokeAllSessions(int accountId, DateTime now, Mutate<AuthSessionEntity> mutateSessions)
    {
        var sessions = await mutateSessions.Read
            .Where(s => s.AccountId == accountId && s.RevokedAt == null && s.ExpiresAt > now)
            .ToListAsync();

        foreach (var session in sessions)
        {
            session.RevokedAt = now;
        }
    }

    /// <summary>Drops rows that can never authenticate again and are older than <paramref name="retention"/>.</summary>
    public static Task<int> DeleteDeadSessions(DateTime now, TimeSpan retention,
        Mutate<AuthSessionEntity> mutateSessions)
    {
        var cutoff = now - retention;
        return mutateSessions.Read
            .Where(s => s.ExpiresAt < cutoff || s.RevokedAt < cutoff)
            .ExecuteDeleteAsync();
    }

    public static string HashToken(string token) =>
        System.Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(token)));

    private static string ToTransport(AuthType authType) => authType switch
    {
        AuthType.TokenSession => AuthSessionEntity.TransportToken,
        AuthType.CookieSession => AuthSessionEntity.TransportCookie,
        _ => throw new ArgumentOutOfRangeException(nameof(authType), authType, null)
    };

    /// <summary>32 random bytes, base64url; for sessions and emailed links.</summary>
    public static string NewToken() => Base64Url(RandomNumberGenerator.GetBytes(32));

    private static string Base64Url(byte[] bytes) =>
        System.Convert.ToBase64String(bytes).TrimEnd('=').Replace('+', '-').Replace('/', '_');
}
