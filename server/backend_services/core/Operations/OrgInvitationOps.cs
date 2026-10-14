using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class OrgInvitationOps
{
    public static readonly TimeSpan Lifetime = TimeSpan.FromDays(7);

    /// <summary>
    /// Returns the raw token for the email; only its hash is stored. An earlier pending invitation of the same
    /// email to the same organization is revoked, so only the latest link works.
    /// </summary>
    public static async Task<(string Token, OrgInvitationEntity Invitation)> Create(
        OrganizationEntity organization,
        string email,
        OrgRole role,
        AccountEntity invitedBy,
        DateTime now,
        Mutate<OrgInvitationEntity> mutateInvitations)
    {
        var normalized = AccountOps.NormalizeLogin(email);
        var earlier = await mutateInvitations.Read
            .Where(i => i.OrganizationId == organization.Id && i.Email == normalized &&
                        i.AcceptedAt == null && i.RevokedAt == null)
            .ToListAsync();
        foreach (var invitation in earlier)
        {
            invitation.RevokedAt = now;
        }

        var token = AuthSessionOps.NewToken();
        var created = new OrgInvitationEntity
        {
            Organization = organization,
            Email = normalized,
            Role = OrgRoles.ToValue(role),
            TokenHash = AuthSessionOps.HashToken(token),
            InvitedBy = invitedBy,
            CreatedAt = now,
            ExpiresAt = now + Lifetime,
        };
        await mutateInvitations.AddAsync(created);
        return (token, created);
    }

    public static bool IsPending(OrgInvitationEntity invitation, DateTime now) =>
        invitation.AcceptedAt == null && invitation.RevokedAt == null && invitation.ExpiresAt > now;

    /// <summary>The pending invitation behind the link, tracked; null when unknown, used, revoked or expired.</summary>
    public static async Task<OrgInvitationEntity?> TryGetLive(
        string token,
        DateTime now,
        Read<OrgInvitationEntity> readInvitations)
    {
        var hash = AuthSessionOps.HashToken(token);
        var invitation = await readInvitations.FirstOrDefaultAsync(i => i.TokenHash == hash);
        return invitation != null && IsPending(invitation, now) ? invitation : null;
    }

    /// <summary>Joins <paramref name="account"/> to the organization with the invited role.</summary>
    public static async Task<OrgMemberEntity> Accept(
        OrgInvitationEntity invitation,
        AccountEntity account,
        DateTime now,
        Mutate<OrgMemberEntity> mutateMembers)
    {
        invitation.AcceptedAt = now;
        return await OrgMemberOps.Add(invitation.OrganizationId, account, OrgRoles.Parse(invitation.Role), now,
            mutateMembers);
    }
}
