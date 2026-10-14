using core.Libs.EntityAccess;
using core.Primitive;
using core.Rules;
using core.Validations;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

/// <summary>What the caller may do in the organization, so the UI hides the rest.</summary>
public class OrgPermissionsPresentation
{
    public required bool CanManageOrganization { get; set; }
    public required bool CanManageMembers { get; set; }
    public required bool CanCreateTeam { get; set; }
    public required bool CanCreateProject { get; set; }

    /// <summary>Roles the caller may invite with or assign.</summary>
    public required List<string> AssignableRoles { get; set; }
}

public class OrganizationPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string Slug { get; set; }
    public required DateTime CreatedAt { get; set; }

    /// <summary>The caller's role.</summary>
    public required string Role { get; set; }

    public required OrgPermissionsPresentation Permissions { get; set; }
}

public class MemberPresentation
{
    public required Guid AccountId { get; set; }
    public required string Email { get; set; }
    public required string Name { get; set; }
    public required string Role { get; set; }
    public required DateTime JoinedAt { get; set; }
    public required bool IsMe { get; set; }
    public required bool CanChangeRole { get; set; }
    public required bool CanRemove { get; set; }
}

public static class OrganizationPresenter
{
    public static OrganizationPresentation PresentOrganization(OrgAccess access)
    {
        var role = access.Role;
        return new OrganizationPresentation
        {
            Id = access.Organization.Id,
            Name = access.Organization.Name,
            Slug = access.Organization.Slug,
            CreatedAt = access.Organization.CreatedAt,
            Role = OrgRoles.ToValue(role),
            Permissions = new OrgPermissionsPresentation
            {
                CanManageOrganization = OrgPermissions.CanManageOrganization(role),
                CanManageMembers = OrgPermissions.CanManageMembers(role),
                CanCreateTeam = OrgPermissions.CanCreateTeam(role),
                CanCreateProject = OrgPermissions.CanCreateProject(role),
                AssignableRoles = OrgPermissions.AssignableRoles(role).Select(OrgRoles.ToValue).ToList(),
            },
        };
    }

    /// <summary>The organization's members by name. The last owner shows as not removable or demotable.</summary>
    public static async Task<List<MemberPresentation>> PresentMembers(
        OrgAccess access,
        Read<OrgMemberEntity> readMembers,
        Read<AccountEntity> readAccounts,
        CancellationToken cancellationToken)
    {
        var members = await readMembers
            .Where(m => m.OrganizationId == access.Organization.Id)
            .Join(readAccounts, m => m.AccountId, a => a.Id, (m, a) => new { Member = m, Account = a })
            .OrderBy(x => x.Account.Name)
            .ThenBy(x => x.Account.Login)
            .ToListAsync(cancellationToken);

        var role = access.Role;
        var owners = members.Count(x => OrgRoles.Parse(x.Member.Role) == OrgRole.Owner);
        return members.Select(x =>
        {
            var targetRole = OrgRoles.Parse(x.Member.Role);
            var isMe = x.Member.AccountId == access.Member.AccountId;
            var isLastOwner = targetRole == OrgRole.Owner && owners == 1;
            return new MemberPresentation
            {
                AccountId = x.Account.ExternalId,
                Email = x.Account.Login,
                Name = x.Account.Name,
                Role = OrgRoles.ToValue(targetRole),
                JoinedAt = x.Member.JoinedAt,
                IsMe = isMe,
                CanChangeRole = !isLastOwner && OrgPermissions.CanManageMember(role, targetRole),
                CanRemove = !isLastOwner && OrgPermissions.CanRemoveMember(role, targetRole, isMe),
            };
        }).ToList();
    }
}
