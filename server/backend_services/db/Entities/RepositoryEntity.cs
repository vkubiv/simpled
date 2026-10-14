using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>A GitHub repository linked to a project; its envspecs become environments and deployments.</summary>
[Table("repositories")]
[Index(nameof(ProjectId), nameof(Owner), nameof(Name), IsUnique = true)]
[Index(nameof(GitHubConnectionId))]
public class RepositoryEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("project_id")]
    public int ProjectId { get; set; }

    [ForeignKey("ProjectId")]
    public ProjectEntity? Project { get; set; }

    [Column("github_connection_id")]
    public int GitHubConnectionId { get; set; }

    [ForeignKey("GitHubConnectionId")]
    public GitHubConnectionEntity? GitHubConnection { get; set; }

    [Column("owner")]
    public required string Owner { get; set; }

    [Column("name")]
    public required string Name { get; set; }

    [Column("default_branch")]
    public required string DefaultBranch { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    [Column("synced_at")]
    public DateTime? SyncedAt { get; set; }

    /// <summary>The git tree the last sync read.</summary>
    [Column("synced_revision")]
    public string? SyncedRevision { get; set; }

    /// <summary>Why the last sync failed, or which spec files it could not read; null when it was clean.</summary>
    [Column("sync_error")]
    public string? SyncError { get; set; }

    public ICollection<DeployEnvironmentEntity>? Environments { get; set; }
}
