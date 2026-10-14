using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

[Table("teams")]
[Index(nameof(OrganizationId), nameof(Name), IsUnique = true)]
public class TeamEntity
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

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    public ICollection<TeamMemberEntity>? Members { get; set; }

    public ICollection<ProjectTeamEntity>? Projects { get; set; }
}
