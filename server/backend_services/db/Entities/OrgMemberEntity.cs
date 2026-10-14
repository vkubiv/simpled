using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

[Table("org_members")]
[Index(nameof(OrganizationId), nameof(AccountId), IsUnique = true)]
[Index(nameof(AccountId))]
public class OrgMemberEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("organization_id")]
    public int OrganizationId { get; set; }

    [ForeignKey("OrganizationId")]
    public OrganizationEntity? Organization { get; set; }

    [Column("account_id")]
    public int AccountId { get; set; }

    [ForeignKey("AccountId")]
    public AccountEntity? Account { get; set; }

    /// <summary>core.Primitive.OrgRole in kebab-case: owner, admin, manager, member.</summary>
    [Column("role")]
    public required string Role { get; set; }

    [Column("joined_at")]
    public required DateTime JoinedAt { get; set; }
}
