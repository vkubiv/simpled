using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.AspNetCore.Identity;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class AccountOps
{
    public const int MaxFailedLogins = 5;
    public static readonly TimeSpan LockoutDuration = TimeSpan.FromMinutes(15);

    /// <summary>Logins are emails, compared case-insensitively by storing them lower-cased.</summary>
#pragma warning disable CA1308 // Emails are conventionally lower case.
    public static string NormalizeLogin(string login) => login.Trim().ToLowerInvariant();
#pragma warning restore CA1308

    public static Task<AccountEntity?> TryGetAccountById(int accountId, Read<AccountEntity> readAccounts) =>
        readAccounts.FirstOrDefaultAsync(x => x.Id == accountId);

    public static Task<AccountEntity?> TryGetAccountByExternalId(Guid externalId, Read<AccountEntity> readAccounts) =>
        readAccounts.FirstOrDefaultAsync(x => x.ExternalId == externalId);

    public static Task<AccountEntity?> TryGetAccountByLogin(string login, Read<AccountEntity> readAccounts)
    {
        var normalized = NormalizeLogin(login);
        return readAccounts.FirstOrDefaultAsync(x => x.Login == normalized);
    }

    public static async Task<AccountEntity> CreateAccount(
        string login,
        string name,
        string password,
        bool confirmed,
        DateTime now,
        Mutate<AccountEntity> mutateAccounts)
    {
        var account = new AccountEntity
        {
            ExternalId = Guid.NewGuid(),
            Login = NormalizeLogin(login),
            Name = name.Trim(),
            Confirmed = confirmed,
            CreatedAt = now,
        };
        SetPassword(account, password);

        await mutateAccounts.AddAsync(account);
        return account;
    }

    public static void SetPassword(AccountEntity account, string password)
    {
        account.Password = new PasswordHasher<Credentials>()
            .HashPassword(new Credentials { Login = account.Login, Password = password }, password);
        account.FailedLoginCount = 0;
        account.LockedUntil = null;
    }

    /// <summary>
    /// The account behind <paramref name="credentials"/>, or null. <see cref="MaxFailedLogins"/> consecutive
    /// failures lock the account for <see cref="LockoutDuration"/>; a locked account rejects even the right
    /// password, with the same null, so the caller cannot tell. The caller saves in both outcomes.
    /// </summary>
    public static async Task<AccountEntity?> TryLogin(
        Credentials credentials,
        DateTime now,
        Mutate<AccountEntity> mutateAccounts)
    {
        var login = NormalizeLogin(credentials.Login);
        var account = await mutateAccounts.Read.FirstOrDefaultAsync(x => x.Login == login);
        if (account == null || account.LockedUntil > now)
        {
            return null;
        }

        if (!VerifyPassword(account, credentials.Password))
        {
            account.FailedLoginCount++;
            if (account.FailedLoginCount >= MaxFailedLogins)
            {
                account.LockedUntil = now + LockoutDuration;
                account.FailedLoginCount = 0;
            }

            return null;
        }

        account.FailedLoginCount = 0;
        account.LockedUntil = null;
        return account;
    }

    private static bool VerifyPassword(AccountEntity account, string password) =>
        account.Password != null &&
        new PasswordHasher<Credentials>().VerifyHashedPassword(
            new Credentials { Login = account.Login, Password = password }, account.Password, password)
        != PasswordVerificationResult.Failed;
}
