using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

[Table("projects")]
[Index(nameof(OrganizationId), nameof(Slug), IsUnique = true)]
public class ProjectEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("organization_id")]
    public int OrganizationId { get; set; }

    [ForeignKey("OrganizationId")]
    public OrganizationEntity? Organization { get; set; }

    [Column("name")]
    public required string Name { get; set; }

    /// <summary>Unique within the organization; what the CLI will address the project by.</summary>
    [Column("slug")]
    public required string Slug { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    public ICollection<ProjectTeamEntity>? Teams { get; set; }
}
