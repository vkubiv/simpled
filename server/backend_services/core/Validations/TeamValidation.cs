using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Validations;

public static class TeamValidation
{
    public const int MaxNameLength = 60;

    public static bool ValidateName(string? name, ValidationResult vr) =>
        StringValidation.ValidateString(name?.Trim(), vr, "name", maxLength: MaxNameLength);

    public static async Task<bool> EnsureNameFree(int organizationId, string name, int? exceptTeamId,
        ValidationResult vr, Read<TeamEntity> readTeams)
    {
        var trimmed = name.Trim();
        if (!await readTeams.AnyAsync(t =>
                t.OrganizationId == organizationId && t.Name == trimmed && t.Id != exceptTeamId))
        {
            return true;
        }

        vr.AlreadyExists("name", "A team with this name already exists.");
        return false;
    }

    /// <summary>
    /// The team, tracked. Answers <c>doesnt-exist</c> like a team of another organization would; the caller then
    /// checks access to <see cref="TeamEntity.OrganizationId"/> with the same field name.
    /// </summary>
    public static async Task<TeamEntity?> TryGetTeam(int teamId, ValidationResult vr, Read<TeamEntity> readTeams,
        string fieldName = "teamId")
    {
        var team = await TeamOps.TryGet(teamId, readTeams);
        if (team == null)
        {
            vr.DoesntExist(fieldName, "Not found.");
        }

        return team;
    }

    /// <summary>
    /// The team (tracked), the caller's access to its organization, and whether the caller is in the team.
    /// A team of an organization the caller is not in answers <c>doesnt-exist</c> on "teamId".
    /// </summary>
    public static async Task<(TeamEntity Team, OrgAccess Access, bool IsTeamMember)?> TryGetTeamAccess(
        int teamId,
        int accountId,
        ValidationResult vr,
        Read<TeamEntity> readTeams,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        Read<TeamMemberEntity> readTeamMembers)
    {
        var team = await TryGetTeam(teamId, vr, readTeams);
        var access = team == null
            ? null
            : await OrgValidation.TryGetAccess(team.OrganizationId, accountId, vr, readMembers, readOrganizations,
                "teamId");
        if (access == null)
        {
            return null;
        }

        return (team!, access, await TeamOps.IsMember(team!.Id, accountId, readTeamMembers));
    }

    /// <summary>All <paramref name="teamIds"/> must be teams of the organization.</summary>
    public static async Task<List<TeamEntity>?> TryGetTeams(int organizationId, IReadOnlyCollection<int> teamIds,
        ValidationResult vr, Read<TeamEntity> readTeams, string fieldName = "teamIds")
    {
        var teams = await readTeams
            .Where(t => t.OrganizationId == organizationId && teamIds.Contains(t.Id))
            .ToListAsync();
        if (teams.Count != teamIds.Distinct().Count())
        {
            vr.DoesntExist(fieldName, "Some of the teams don't exist.");
            return null;
        }

        return teams;
    }
}
