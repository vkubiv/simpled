using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>
/// A deployment of an envspec as of the last repository sync, and what the monitor last saw at its version
/// endpoint (`/.well-known/simpled/version` on the primary host).
/// </summary>
[Table("deployments")]
[Index(nameof(EnvironmentId), nameof(Name), IsUnique = true)]
public class DeploymentEntity
{
    public const string StatusUnknown = "unknown";
    public const string StatusUp = "up";
    public const string StatusDown = "down";

    /// <summary>The endpoint answers for another application or deployment.</summary>
    public const string StatusMismatch = "mismatch";

    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("environment_id")]
    public int EnvironmentId { get; set; }

    [ForeignKey("EnvironmentId")]
    public DeployEnvironmentEntity? Environment { get; set; }

    [Column("name")]
    public required string Name { get; set; }

    [Column("application_name")]
    public required string ApplicationName { get; set; }

    /// <summary>The envspec's `application.version` constraint, such as ^1.2.0.</summary>
    [Column("version_constraint")]
    public string? VersionConstraint { get; set; }

    /// <summary>The primary host's base URL; null when the envspec gives none the monitor can reach.</summary>
    [Column("url")]
    public string? Url { get; set; }

    /// <summary>The `version` of the application's appspec.yaml on the default branch: what ships next.</summary>
    [Column("repo_version")]
    public string? RepoVersion { get; set; }

    [Column("status")]
    public required string Status { get; set; }

    [Column("status_since")]
    public DateTime? StatusSince { get; set; }

    [Column("live_version")]
    public string? LiveVersion { get; set; }

    /// <summary>When `simpled prepare-deployment` made the running stack, from the version endpoint.</summary>
    [Column("live_deployed_at")]
    public DateTime? LiveDeployedAt { get; set; }

    [Column("checked_at")]
    public DateTime? CheckedAt { get; set; }

    [Column("check_error")]
    public string? CheckError { get; set; }

    /// <summary>Failed checks in a row; one failure is a blip, a few make the deployment down.</summary>
    [Column("failure_count")]
    public int FailureCount { get; set; }
}
