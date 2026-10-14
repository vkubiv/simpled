using core.Libs.EntityAccess;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class TeamOps
{
    public static Task<TeamEntity?> TryGet(int teamId, Read<TeamEntity> readTeams) =>
        readTeams.FirstOrDefaultAsync(t => t.Id == teamId);

    public static async Task<TeamEntity> Create(
        int organizationId,
        string name,
        DateTime now,
        Mutate<TeamEntity> mutateTeams)
    {
        var team = new TeamEntity
        {
            OrganizationId = organizationId,
            Name = name.Trim(),
            CreatedAt = now,
        };
        await mutateTeams.AddAsync(team);
        return team;
    }

    /// <summary>Removes the team; the database cascades to its members and project links.</summary>
    public static void Delete(TeamEntity team, Mutate<TeamEntity> mutateTeams) => mutateTeams.Remove(team);

    public static Task<bool> IsMember(int teamId, int accountId, Read<TeamMemberEntity> readTeamMembers) =>
        readTeamMembers.AnyAsync(tm => tm.TeamId == teamId && tm.AccountId == accountId);

    /// <summary>The ids of <paramref name="organizationId"/>'s teams that the account belongs to.</summary>
    public static Task<List<int>> TeamIdsOf(
        int organizationId,
        int accountId,
        Read<TeamEntity> readTeams,
        Read<TeamMemberEntity> readTeamMembers) =>
        readTeamMembers
            .Where(tm => tm.AccountId == accountId)
            .Join(readTeams.Where(t => t.OrganizationId == organizationId), tm => tm.TeamId, t => t.Id,
                (_, t) => t.Id)
            .ToListAsync();

    public static async Task<TeamMemberEntity> AddMember(
        TeamEntity team,
        AccountEntity account,
        DateTime now,
        Mutate<TeamMemberEntity> mutateTeamMembers)
    {
        var member = new TeamMemberEntity { Team = team, Account = account, AddedAt = now };
        await mutateTeamMembers.AddAsync(member);
        return member;
    }

    public static Task<TeamMemberEntity?> TryGetMember(int teamId, int accountId,
        Read<TeamMemberEntity> readTeamMembers) =>
        readTeamMembers.FirstOrDefaultAsync(tm => tm.TeamId == teamId && tm.AccountId == accountId);
}
