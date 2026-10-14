using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

[Table("accounts")]
[Index(nameof(Login), IsUnique = true)]
[Index(nameof(ExternalId), IsUnique = true)]
public class AccountEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("external_id")]
    public required Guid ExternalId { get; set; }

    /// <summary>The email address, lower-cased.</summary>
    [Column("login")]
    public required string Login { get; set; }

    [Column("name")]
    public required string Name { get; set; }

    [Column("password")]
    public string? Password { get; set; }

    [Column("confirmed")]
    public required bool Confirmed { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    /// <summary>Consecutive failed logins since the last success or lockout.</summary>
    [Column("failed_login_count")]
    public int FailedLoginCount { get; set; }

    [Column("locked_until")]
    public DateTime? LockedUntil { get; set; }
}
