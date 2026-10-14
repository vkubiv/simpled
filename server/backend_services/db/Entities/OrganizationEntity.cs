using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

[Table("organizations")]
[Index(nameof(Slug), IsUnique = true)]
[Index(nameof(ExternalId), IsUnique = true)]
public class OrganizationEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("external_id")]
    public required Guid ExternalId { get; set; }

    [Column("name")]
    public required string Name { get; set; }

    [Column("slug")]
    public required string Slug { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    public ICollection<OrgMemberEntity>? Members { get; set; }
}
