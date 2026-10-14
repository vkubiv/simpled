using core.Primitive;
using core.Rules;

namespace utest.Core.Rules;

public class TestOrgPermissions
{
    [Theory]
    [InlineData(OrgRole.Owner, true)]
    [InlineData(OrgRole.Admin, false)]
    [InlineData(OrgRole.Manager, false)]
    [InlineData(OrgRole.Member, false)]
    public void TestCanManageOrganization(OrgRole role, bool allowed)
    {
        Assert.Equal(allowed, OrgPermissions.CanManageOrganization(role));
    }

    [Theory]
    [InlineData(OrgRole.Owner, OrgRole.Owner, OrgRole.Member, true)]
    [InlineData(OrgRole.Owner, OrgRole.Member, OrgRole.Owner, true)]
    [InlineData(OrgRole.Admin, OrgRole.Member, OrgRole.Admin, true)]
    [InlineData(OrgRole.Admin, OrgRole.Member, OrgRole.Owner, false)]
    [InlineData(OrgRole.Admin, OrgRole.Owner, OrgRole.Member, false)]
    [InlineData(OrgRole.Manager, OrgRole.Member, OrgRole.Manager, false)]
    [InlineData(OrgRole.Member, OrgRole.Member, OrgRole.Member, false)]
    public void TestCanChangeRole(OrgRole role, OrgRole targetRole, OrgRole newRole, bool allowed)
    {
        Assert.Equal(allowed, OrgPermissions.CanChangeRole(role, targetRole, newRole));
    }

    [Theory]
    [InlineData(OrgRole.Member, OrgRole.Member, true, true)]
    [InlineData(OrgRole.Member, OrgRole.Member, false, false)]
    [InlineData(OrgRole.Admin, OrgRole.Manager, false, true)]
    [InlineData(OrgRole.Admin, OrgRole.Owner, false, false)]
    [InlineData(OrgRole.Owner, OrgRole.Owner, false, true)]
    public void TestCanRemoveMember(OrgRole role, OrgRole targetRole, bool isSelf, bool allowed)
    {
        Assert.Equal(allowed, OrgPermissions.CanRemoveMember(role, targetRole, isSelf));
    }

    [Theory]
    [InlineData(OrgRole.Owner, false, true)]
    [InlineData(OrgRole.Admin, false, true)]
    [InlineData(OrgRole.Manager, true, true)]
    [InlineData(OrgRole.Manager, false, false)]
    [InlineData(OrgRole.Member, true, false)]
    public void TestCanManageTeamAndProject(OrgRole role, bool isMember, bool allowed)
    {
        Assert.Equal(allowed, OrgPermissions.CanManageTeam(role, isMember));
        Assert.Equal(allowed, OrgPermissions.CanManageProject(role, isMember));
    }

    [Fact]
    public void TestCreate_MembersCannot()
    {
        Assert.True(OrgPermissions.CanCreateTeam(OrgRole.Manager));
        Assert.True(OrgPermissions.CanCreateProject(OrgRole.Manager));
        Assert.False(OrgPermissions.CanCreateTeam(OrgRole.Member));
        Assert.False(OrgPermissions.CanCreateProject(OrgRole.Member));
    }
}
