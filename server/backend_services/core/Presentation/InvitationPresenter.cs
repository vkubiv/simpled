using core.Libs.EntityAccess;
using core.Operations;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

public class InvitationPresentation
{
    public required int Id { get; set; }
    public required string Email { get; set; }
    public required string Role { get; set; }
    public required string InvitedBy { get; set; }
    public required DateTime CreatedAt { get; set; }
    public required DateTime ExpiresAt { get; set; }
    public required bool Expired { get; set; }
}

/// <summary>What the invitation page shows before the person accepts or registers.</summary>
public class InvitationPreviewPresentation
{
    public required string OrganizationName { get; set; }
    public required string Role { get; set; }
    public required string Email { get; set; }
    public required string InvitedBy { get; set; }

    /// <summary>True: sign in and accept. False: register through the invitation.</summary>
    public required bool AccountExists { get; set; }
}

public static class InvitationPresenter
{
    /// <summary>Invitations neither accepted nor revoked, newest first; expired ones are flagged for a resend.</summary>
    public static async Task<List<InvitationPresentation>> PresentOpen(
        int organizationId,
        DateTime now,
        Read<OrgInvitationEntity> readInvitations,
        Read<AccountEntity> readAccounts,
        CancellationToken cancellationToken)
    {
        var invitations = await readInvitations
            .Where(i => i.OrganizationId == organizationId && i.AcceptedAt == null && i.RevokedAt == null)
            .Join(readAccounts, i => i.InvitedByAccountId, a => a.Id, (i, a) => new { Invitation = i, a.Name })
            .OrderByDescending(x => x.Invitation.CreatedAt)
            .ToListAsync(cancellationToken);

        return invitations.Select(x => PresentInvitation(x.Invitation, x.Name, now)).ToList();
    }

    public static InvitationPresentation PresentInvitation(OrgInvitationEntity invitation, string invitedBy,
        DateTime now) => new()
    {
        Id = invitation.Id,
        Email = invitation.Email,
        Role = invitation.Role,
        InvitedBy = invitedBy,
        CreatedAt = invitation.CreatedAt,
        ExpiresAt = invitation.ExpiresAt,
        Expired = invitation.ExpiresAt <= now,
    };

    public static async Task<InvitationPreviewPresentation> PresentPreview(
        OrgInvitationEntity invitation,
        Read<OrganizationEntity> readOrganizations,
        Read<AccountEntity> readAccounts,
        CancellationToken cancellationToken)
    {
        var organization = await readOrganizations.FirstAsync(o => o.Id == invitation.OrganizationId,
            cancellationToken);
        var invitedBy = await readAccounts.FirstAsync(a => a.Id == invitation.InvitedByAccountId,
            cancellationToken);
        return new InvitationPreviewPresentation
        {
            OrganizationName = organization.Name,
            Role = OrgRoles.ToValue(OrgRoles.Parse(invitation.Role)),
            Email = invitation.Email,
            InvitedBy = invitedBy.Name,
            AccountExists = await AccountOps.TryGetAccountByLogin(invitation.Email, readAccounts) != null,
        };
    }
}
