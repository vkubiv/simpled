using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>
/// A single-use token mailed to someone: email confirmation, password reset. Only its SHA-256 is stored.
/// Written only by ConfirmationTokenOps.
/// </summary>
[Table("confirmation_tokens")]
[Index(nameof(TokenHash), IsUnique = true)]
[Index(nameof(AccountId))]
public class ConfirmationTokenEntity
{
    public const string PurposeEmailConfirmation = "email-confirmation";
    public const string PurposeResetPassword = "reset-password";

    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public long Id { get; set; }

    [Column("token_hash")]
    public required string TokenHash { get; set; }

    [Column("purpose")]
    public required string Purpose { get; set; }

    [Column("account_id")]
    public int AccountId { get; set; }

    [ForeignKey("AccountId")]
    public AccountEntity? Account { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    [Column("expires_at")]
    public required DateTime ExpiresAt { get; set; }

    [Column("used_at")]
    public DateTime? UsedAt { get; set; }
}
