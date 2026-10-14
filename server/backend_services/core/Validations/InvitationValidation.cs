using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Validations;

public static class InvitationValidation
{
    public static async Task<bool> EnsureNotMember(
        int organizationId,
        string email,
        ValidationResult vr,
        Read<AccountEntity> readAccounts,
        Read<OrgMemberEntity> readMembers)
    {
        var account = await AccountOps.TryGetAccountByLogin(email, readAccounts);
        if (account == null || await OrgMemberOps.TryGet(organizationId, account.Id, readMembers) == null)
        {
            return true;
        }

        vr.AlreadyExists("email", "This person is already a member of the organization.");
        return false;
    }

    /// <summary>An invitation of the organization, neither accepted nor revoked; tracked.</summary>
    public static async Task<OrgInvitationEntity?> TryGetOpen(int organizationId, int invitationId,
        ValidationResult vr, Read<OrgInvitationEntity> readInvitations)
    {
        var invitation = await readInvitations.FirstOrDefaultAsync(i =>
            i.Id == invitationId && i.OrganizationId == organizationId && i.AcceptedAt == null && i.RevokedAt == null);
        if (invitation == null)
        {
            vr.DoesntExist("invitationId", "Not found.");
        }

        return invitation;
    }

    public static async Task<OrgInvitationEntity?> TryGetLive(
        string token,
        DateTime now,
        ValidationResult vr,
        Read<OrgInvitationEntity> readInvitations)
    {
        var invitation = await OrgInvitationOps.TryGetLive(token, now, readInvitations);
        if (invitation == null)
        {
            vr.IncorrectValue("token", "The invitation is invalid, used or has expired.");
        }

        return invitation;
    }
}
