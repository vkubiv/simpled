using System.ComponentModel.DataAnnotations;
using System.ComponentModel.DataAnnotations.Schema;
using Microsoft.EntityFrameworkCore;

namespace db.Entities;

/// <summary>An organization member in a team; removed with their org membership.</summary>
[Table("team_members")]
[Index(nameof(TeamId), nameof(AccountId), IsUnique = true)]
[Index(nameof(AccountId))]
public class TeamMemberEntity
{
    [Column("id")]
    [Key]
    [DatabaseGenerated(DatabaseGeneratedOption.Identity)]
    public int Id { get; set; }

    [Column("team_id")]
    public int TeamId { get; set; }

    [ForeignKey("TeamId")]
    public TeamEntity? Team { get; set; }

    [Column("account_id")]
    public int AccountId { get; set; }

    [ForeignKey("AccountId")]
    public AccountEntity? Account { get; set; }

    [Column("added_at")]
    public required DateTime AddedAt { get; set; }
}
