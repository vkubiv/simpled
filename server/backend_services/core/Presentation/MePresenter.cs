using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

public class AccountPresentation
{
    public required Guid Id { get; set; }
    public required string Email { get; set; }
    public required string Name { get; set; }
}

public class MyOrganizationPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string Slug { get; set; }
    public required string Role { get; set; }
}

public class MePresentation
{
    public required AccountPresentation Account { get; set; }
    public required List<MyOrganizationPresentation> Organizations { get; set; }
}

public static class MePresenter
{
    public static async Task<MePresentation> PresentMe(
        AccountEntity account,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        CancellationToken cancellationToken)
    {
        var memberships = await readMembers
            .Where(m => m.AccountId == account.Id)
            .Join(readOrganizations, m => m.OrganizationId, o => o.Id, (m, o) => new { m.Role, Organization = o })
            .OrderBy(x => x.Organization.Name)
            .ToListAsync(cancellationToken);

        return new MePresentation
        {
            Account = PresentAccount(account),
            Organizations = memberships.Select(x => PresentMyOrganization(x.Organization, x.Role)).ToList(),
        };
    }

    public static MyOrganizationPresentation PresentMyOrganization(OrganizationEntity organization,
        OrgMemberEntity member) => PresentMyOrganization(organization, member.Role);

    private static MyOrganizationPresentation PresentMyOrganization(OrganizationEntity organization, string role) =>
        new()
        {
            Id = organization.Id,
            Name = organization.Name,
            Slug = organization.Slug,
            Role = OrgRoles.ToValue(OrgRoles.Parse(role)),
        };

    public static AccountPresentation PresentAccount(AccountEntity account) => new()
    {
        Id = account.ExternalId,
        Email = account.Login,
        Name = account.Name,
    };
}
