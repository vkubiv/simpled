using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using utest.TestSupport;

namespace utest.Core.Operations;

public class TestConfirmationTokenOps
{
    private static readonly DateTime T0 = new(2026, 10, 1, 12, 0, 0, DateTimeKind.Utc);

    [Fact]
    public async Task TestTryRedeem_ExpiredOrOtherPurpose()
    {
        var db = InMemoryPrimaryDb.Create();
        var account = await AccountSupport.CreateAccount(db);
        var mutate = new Mutate<ConfirmationTokenEntity>(db);
        var token = await ConfirmationTokenOps.Create(account, ConfirmationTokenEntity.PurposeResetPassword, T0, mutate);
        await db.SaveChangesAsync();

        Assert.Null(await ConfirmationTokenOps.TryRedeem(token, ConfirmationTokenEntity.PurposeEmailConfirmation,
            T0, mutate));
        Assert.Null(await ConfirmationTokenOps.TryRedeem(token, ConfirmationTokenEntity.PurposeResetPassword,
            T0 + TimeSpan.FromHours(1), mutate));

        var redeemed = await ConfirmationTokenOps.TryRedeem(token, ConfirmationTokenEntity.PurposeResetPassword,
            T0 + TimeSpan.FromMinutes(59), mutate);
        Assert.Equal(account.Id, redeemed?.AccountId);
        Assert.NotEqual(token, redeemed!.TokenHash);
    }
}
