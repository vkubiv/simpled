using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>
/// One login. The client holds a random token (bearer header or cookie); only its SHA-256 is stored, so a
/// database leak does not hand out sessions. Written only by AuthSessionOps.
/// </summary>
[Table("auth_sessions")]
[Index(nameof(TokenHash), IsUnique = true)]
[Index(nameof(AccountId))]
public class AuthSessionEntity
{
    public const string AudienceMain = "main";

    public const string TransportToken = "token";
    public const string TransportCookie = "cookie";

    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public long Id { get; set; }

    [Column("token_hash")]
    public required string TokenHash { get; set; }

    [Column("account_id")]
    public int AccountId { get; set; }

    [ForeignKey("AccountId")]
    public AccountEntity? Account { get; set; }

    /// <summary>Service the session was issued by; a session never authenticates against another one.</summary>
    [Column("audience")]
    public required string Audience { get; set; }

    [Column("transport")]
    public required string Transport { get; set; }

    [Column("created_at")]
    public required DateTime CreatedAt { get; set; }

    [Column("last_seen_at")]
    public required DateTime LastSeenAt { get; set; }

    [Column("idle_timeout")]
    public required TimeSpan IdleTimeout { get; set; }

    /// <summary>Absolute end of the session, however active it is.</summary>
    [Column("expires_at")]
    public required DateTime ExpiresAt { get; set; }

    [Column("revoked_at")]
    public DateTime? RevokedAt { get; set; }

    [Column("user_agent")]
    public string? UserAgent { get; set; }
}
