using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>
/// An emailed invitation to join an organization with a role. The link carries a random token; only its SHA-256
/// is stored. Written only by OrgInvitationOps.
/// </summary>
[Table("org_invitations")]
[Index(nameof(TokenHash), IsUnique = true)]
[Index(nameof(OrganizationId), nameof(Email))]
public class OrgInvitationEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("organization_id")]
    public int OrganizationId { get; set; }

    [ForeignKey("OrganizationId")]
    public OrganizationEntity? Organization { get; set; }

    /// <summary>Lower-cased, like accounts.login.</summary>
    [Column("email")]
    public required string Email { get; set; }

    /// <summary>core.Primitive.OrgRole in kebab-case.</summary>
    [Column("role")]
    public required string Role { get; set; }

    [Column("token_hash")]
    public required string TokenHash { get; set; }

    [Column("invited_by_account_id")]
    public int InvitedByAccountId { get; set; }

    [ForeignKey("InvitedByAccountId")]
    public AccountEntity? InvitedBy { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    [Column("expires_at")]
    public required DateTime ExpiresAt { get; set; }

    [Column("accepted_at")]
    public DateTime? AcceptedAt { get; set; }

    [Column("revoked_at")]
    public DateTime? RevokedAt { get; set; }
}
