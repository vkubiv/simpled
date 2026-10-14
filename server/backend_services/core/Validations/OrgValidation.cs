using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Validations;

/// <summary>The caller's place in an organization: the organization (tracked) and their membership.</summary>
public record OrgAccess(OrganizationEntity Organization, OrgMemberEntity Member)
{
    public OrgRole Role => OrgRoles.Parse(Member.Role);
}

public static class OrgValidation
{
    /// <summary>
    /// The caller's access to the organization. An organization the caller is not a member of answers
    /// <c>doesnt-exist</c> on <paramref name="fieldName"/>, so ids of other organizations reveal nothing.
    /// </summary>
    public static async Task<OrgAccess?> TryGetAccess(
        int organizationId,
        int accountId,
        ValidationResult vr,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        string fieldName = "orgId")
    {
        var member = await readMembers
            .Include(m => m.Organization, readOrganizations)
            .FirstOrDefaultAsync(m => m.OrganizationId == organizationId && m.AccountId == accountId);
        if (member == null)
        {
            vr.DoesntExist(fieldName, "Not found.");
            return null;
        }

        return new OrgAccess(member.Organization!, member);
    }

    public static bool EnsureAllowed(bool allowed, ValidationResult vr, string fieldName = "role")
    {
        if (!allowed)
        {
            vr.Forbidden(fieldName, "Your role in this organization does not allow this.");
        }

        return allowed;
    }

    public static OrgRole? ValidateRole(string? role, ValidationResult vr, string fieldName = "role")
    {
        if (string.IsNullOrEmpty(role))
        {
            vr.ValueRequired(fieldName, "Role is required.");
            return null;
        }

        var parsed = OrgRoles.TryParse(role);
        if (parsed == null)
        {
            vr.IncorrectValue(fieldName, "Role must be owner, admin, manager or member.");
        }

        return parsed;
    }

    /// <summary>A member of the organization by account id; <c>doesnt-exist</c> on "accountId" otherwise.</summary>
    public static async Task<(AccountEntity Account, OrgMemberEntity Member)?> TryGetMember(
        int organizationId,
        Guid accountId,
        ValidationResult vr,
        Read<AccountEntity> readAccounts,
        Read<OrgMemberEntity> readMembers)
    {
        var account = await AccountOps.TryGetAccountByExternalId(accountId, readAccounts);
        var member = account == null ? null : await OrgMemberOps.TryGet(organizationId, account.Id, readMembers);
        if (member == null)
        {
            vr.DoesntExist("accountId", "Not a member of this organization.");
            return null;
        }

        return (account!, member);
    }

    /// <summary>An organization always keeps an owner: the last one can't be demoted or removed.</summary>
    public static async Task<bool> EnsureNotLastOwner(
        OrgMemberEntity target,
        OrgRole? newRole,
        ValidationResult vr,
        Read<OrgMemberEntity> readMembers)
    {
        if (OrgRoles.Parse(target.Role) != OrgRole.Owner || newRole == OrgRole.Owner ||
            await OrgMemberOps.CountOwners(target.OrganizationId, readMembers) > 1)
        {
            return true;
        }

        vr.IncorrectState("accountId", "An organization needs at least one owner.");
        return false;
    }
}
