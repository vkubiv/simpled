using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>A team linked to a project; the team's members are the project's members.</summary>
[Table("project_teams")]
[Index(nameof(ProjectId), nameof(TeamId), IsUnique = true)]
[Index(nameof(TeamId))]
public class ProjectTeamEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("project_id")]
    public int ProjectId { get; set; }

    [ForeignKey("ProjectId")]
    public ProjectEntity? Project { get; set; }

    [Column("team_id")]
    public int TeamId { get; set; }

    [ForeignKey("TeamId")]
    public TeamEntity? Team { get; set; }
}
