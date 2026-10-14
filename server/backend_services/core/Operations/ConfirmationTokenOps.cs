using core.Libs.EntityAccess;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class ConfirmationTokenOps
{
    public static TimeSpan Lifetime(string purpose) => purpose switch
    {
        ConfirmationTokenEntity.PurposeEmailConfirmation => TimeSpan.FromDays(1),
        ConfirmationTokenEntity.PurposeResetPassword => TimeSpan.FromHours(1),
        _ => throw new ArgumentOutOfRangeException(nameof(purpose), purpose, null),
    };

    /// <summary>
    /// Returns the raw token for the email; only its hash is stored. Earlier unused tokens of the same purpose
    /// stop working, so only the latest email's link is valid. Saved with the caller's unit of work.
    /// </summary>
    public static async Task<string> Create(
        AccountEntity account,
        string purpose,
        DateTime now,
        Mutate<ConfirmationTokenEntity> mutateTokens)
    {
        if (account.Id != 0)
        {
            var earlier = await mutateTokens.Read
                .Where(t => t.AccountId == account.Id && t.Purpose == purpose && t.UsedAt == null)
                .ToListAsync();
            foreach (var token in earlier)
            {
                token.UsedAt = now;
            }
        }

        var raw = AuthSessionOps.NewToken();
        await mutateTokens.AddAsync(new ConfirmationTokenEntity
        {
            TokenHash = AuthSessionOps.HashToken(raw),
            Purpose = purpose,
            Account = account,
            CreatedAt = now,
            ExpiresAt = now + Lifetime(purpose),
        });
        return raw;
    }

    /// <summary>The live token, tracked; null when unknown, used, expired or for another purpose.</summary>
    public static async Task<ConfirmationTokenEntity?> TryGetLive(
        string token,
        string purpose,
        DateTime now,
        Mutate<ConfirmationTokenEntity> mutateTokens)
    {
        var hash = AuthSessionOps.HashToken(token);
        var entity = await mutateTokens.Read.FirstOrDefaultAsync(t => t.TokenHash == hash);
        return entity == null || entity.Purpose != purpose || entity.UsedAt != null || entity.ExpiresAt <= now
            ? null
            : entity;
    }

    /// <summary><see cref="TryGetLive"/>, marked used.</summary>
    public static async Task<ConfirmationTokenEntity?> TryRedeem(
        string token,
        string purpose,
        DateTime now,
        Mutate<ConfirmationTokenEntity> mutateTokens)
    {
        var entity = await TryGetLive(token, purpose, now, mutateTokens);
        if (entity != null)
        {
            entity.UsedAt = now;
        }

        return entity;
    }
}
