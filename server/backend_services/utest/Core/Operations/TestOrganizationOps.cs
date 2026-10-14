using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using utest.TestSupport;

namespace utest.Core.Operations;

public class TestOrganizationOps
{
    [Theory]
    [InlineData("Acme Corp", "acme-corp")]
    [InlineData("  --Ünïcode & Co.  ", "n-code-co")]
    [InlineData("日本", "org")]
    [InlineData("a-very-long-organization-name-that-goes-on-and-on", "a-very-long-organization-name-that-goes")]
    public void TestSlugify(string name, string slug)
    {
        Assert.Equal(slug, OrganizationOps.Slugify(name));
    }

    [Fact]
    public async Task TestCreate_SlugIsUnique()
    {
        var db = InMemoryPrimaryDb.Create();
        var account = await AccountSupport.CreateAccount(db);

        var slugs = new List<string>();
        for (var i = 0; i < 3; i++)
        {
            var (organization, _) = await OrganizationOps.Create("Acme", account, DateTime.UtcNow,
                mutateOrganizations: new Mutate<OrganizationEntity>(db),
                mutateMembers: new Mutate<OrgMemberEntity>(db));
            await db.SaveChangesAsync();
            slugs.Add(organization.Slug);
        }

        Assert.Equal(["acme", "acme-2", "acme-3"], slugs);
    }
}
