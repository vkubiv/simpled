using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>Something the monitor noticed about a deployment: a new version, going down, coming back.</summary>
[Table("deployment_events")]
[Index(nameof(DeploymentId), nameof(At))]
public class DeploymentEventEntity
{
    public const string KindVersionChanged = "version-changed";
    public const string KindDown = "down";
    public const string KindUp = "up";
    public const string KindMismatch = "mismatch";

    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("deployment_id")]
    public int DeploymentId { get; set; }

    [ForeignKey("DeploymentId")]
    public DeploymentEntity? Deployment { get; set; }

    [Column("at")]
    public required DateTime At { get; set; }

    [Column("kind")]
    public required string Kind { get; set; }

    [Column("version")]
    public string? Version { get; set; }

    [Column("previous_version")]
    public string? PreviousVersion { get; set; }

    [Column("details")]
    public string? Details { get; set; }
}
