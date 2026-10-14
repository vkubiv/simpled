using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>A GitHub personal access token an organization reads its repositories with.</summary>
[Table("github_connections")]
[Index(nameof(OrganizationId), nameof(Name), IsUnique = true)]
public class GitHubConnectionEntity
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

    /// <summary>The GitHub account the token belongs to, as GitHub answered when it was saved.</summary>
    [Column("github_login")]
    public required string GitHubLogin { get; set; }

    /// <summary>The token, encrypted with the github_token_key secret (TokenCipher); never sent out.</summary>
    [Column("token_ciphertext")]
    public required string TokenCiphertext { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }
}
