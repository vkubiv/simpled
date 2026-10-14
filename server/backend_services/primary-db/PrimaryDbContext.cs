using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace primary_db;

public class PrimaryDbContext : DbContext
{
    // Accounts
    public DbSet<AccountEntity> Accounts { get; set; }
    public DbSet<AuthSessionEntity> AuthSessions { get; set; }
    public DbSet<ConfirmationTokenEntity> ConfirmationTokens { get; set; }

    // Organizations
    public DbSet<OrganizationEntity> Organizations { get; set; }
    public DbSet<OrgMemberEntity> OrgMembers { get; set; }
    public DbSet<OrgInvitationEntity> OrgInvitations { get; set; }
    public DbSet<TeamEntity> Teams { get; set; }
    public DbSet<TeamMemberEntity> TeamMembers { get; set; }
    public DbSet<ProjectEntity> Projects { get; set; }
    public DbSet<ProjectTeamEntity> ProjectTeams { get; set; }

    // Monitoring
    public DbSet<GitHubConnectionEntity> GitHubConnections { get; set; }
    public DbSet<RepositoryEntity> Repositories { get; set; }
    public DbSet<DeployEnvironmentEntity> Environments { get; set; }
    public DbSet<DeploymentEntity> Deployments { get; set; }
    public DbSet<DeploymentEventEntity> DeploymentEvents { get; set; }

    public PrimaryDbContext(DbContextOptions<PrimaryDbContext> options) : base(options)
    {
    }
}
