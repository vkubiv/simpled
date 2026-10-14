using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using primary_db;

namespace utest.TestSupport;

public static class AccountSupport
{
    public const string Password = "correct-horse-battery";

    public static async Task<AccountEntity> CreateAccount(PrimaryDbContext db, string email = "ann@test.com",
        bool confirmed = true)
    {
        var account = await AccountOps.CreateAccount(
            login: email,
            name: "Ann",
            password: Password,
            confirmed: confirmed,
            now: DateTime.UtcNow,
            mutateAccounts: new Mutate<AccountEntity>(db));
        await db.SaveChangesAsync();
        return account;
    }
}
