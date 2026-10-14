using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>One envspec.yaml of a repository.</summary>
[Table("environments")]
[Index(nameof(RepositoryId), nameof(Path), IsUnique = true)]
public class DeployEnvironmentEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("repository_id")]
    public int RepositoryId { get; set; }

    [ForeignKey("RepositoryId")]
    public RepositoryEntity? Repository { get; set; }

    /// <summary>The envspec's directory name: `deployment/prod/envspec.yaml` is "prod".</summary>
    [Column("name")]
    public required string Name { get; set; }

    /// <summary>The envspec's path in the repository.</summary>
    [Column("path")]
    public required string Path { get; set; }

    public ICollection<DeploymentEntity>? Deployments { get; set; }
}
