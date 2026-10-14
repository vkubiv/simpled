using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class OrganizationOps
{
    /// <summary>Creates the organization with <paramref name="owner"/> as its first owner.</summary>
    public static async Task<(OrganizationEntity Organization, OrgMemberEntity Owner)> Create(
        string name,
        AccountEntity owner,
        DateTime now,
        Mutate<OrganizationEntity> mutateOrganizations,
        Mutate<OrgMemberEntity> mutateMembers)
    {
        var organization = new OrganizationEntity
        {
            ExternalId = Guid.NewGuid(),
            Name = name.Trim(),
            Slug = await UniqueSlug(name, mutateOrganizations.Read),
            CreatedAt = now,
        };
        await mutateOrganizations.AddAsync(organization);

        var member = new OrgMemberEntity
        {
            Organization = organization,
            Account = owner,
            Role = OrgRoles.ToValue(OrgRole.Owner),
            JoinedAt = now,
        };
        await mutateMembers.AddAsync(member);

        return (organization, member);
    }

    public static string Slugify(string name) => Slug.From(name, "org");

    /// <summary>Removes the organization; the database cascades to members, invitations, teams and projects.</summary>
    public static void Delete(OrganizationEntity organization, Mutate<OrganizationEntity> mutateOrganizations) =>
        mutateOrganizations.Remove(organization);

    private static async Task<string> UniqueSlug(string name, Read<OrganizationEntity> readOrganizations)
    {
        var slug = Slugify(name);
        var taken = await readOrganizations
            .Where(o => o.Slug == slug || o.Slug.StartsWith(slug + "-"))
            .Select(o => o.Slug)
            .ToListAsync();
        return Slug.FirstFree(slug, taken);
    }
}
