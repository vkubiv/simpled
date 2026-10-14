using core.Auth;
using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using main_svc.Auth;
using main_svc.Controllers;
using utest.TestSupport;

namespace utest.MainTest;

public class TestMeController
{
    [Fact]
    public async Task TestGet_ListsOrganizationsWithRole()
    {
        var db = InMemoryPrimaryDb.Create();
        var account = await AccountSupport.CreateAccount(db);
        foreach (var name in new[] { "Zeta", "Alpha" })
        {
            await OrganizationOps.Create(name, account, DateTime.UtcNow,
                mutateOrganizations: new Mutate<OrganizationEntity>(db),
                mutateMembers: new Mutate<OrgMemberEntity>(db));
        }

        await db.SaveChangesAsync();

        var result = await new MeController().Get(
            new MainClaims { AccountId = account.Id, SessionId = 1, AuthType = AuthType.CookieSession },
            context: CallContextSupport.Main(db),
            readAccounts: new Read<AccountEntity>(db),
            readMembers: new Read<OrgMemberEntity>(db),
            readOrganizations: new Read<OrganizationEntity>(db));

        var me = result.Value!;
        Assert.Equal("ann@test.com", me.Account.Email);
        Assert.Equal(["Alpha", "Zeta"], me.Organizations.Select(o => o.Name));
        Assert.All(me.Organizations, o => Assert.Equal("owner", o.Role));
    }
}
