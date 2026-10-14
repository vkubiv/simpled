using core.Libs.EntityAccess;
using core.Primitive;
using core.Rules;
using core.Validations;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

public class TeamSummaryPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required int MemberCount { get; set; }
    public required int ProjectCount { get; set; }

    /// <summary>The caller is in the team.</summary>
    public required bool IsMember { get; set; }

    public required bool CanManage { get; set; }
}

public class TeamMemberPresentation
{
    public required Guid AccountId { get; set; }
    public required string Email { get; set; }
    public required string Name { get; set; }

    /// <summary>Their role in the organization.</summary>
    public required string Role { get; set; }
}

public class ProjectRefPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string Slug { get; set; }
}

public class TeamPresentation
{
    public required int Id { get; set; }
    public required int OrganizationId { get; set; }
    public required string Name { get; set; }
    public required DateTime CreatedAt { get; set; }
    public required List<TeamMemberPresentation> Members { get; set; }
    public required List<ProjectRefPresentation> Projects { get; set; }
    public required bool IsMember { get; set; }
    public required bool CanManage { get; set; }
}

public static class TeamPresenter
{
    public static async Task<List<TeamSummaryPresentation>> PresentTeams(
        OrgAccess access,
        Read<TeamEntity> readTeams,
        Read<TeamMemberEntity> readTeamMembers,
        Read<ProjectTeamEntity> readProjectTeams,
        CancellationToken cancellationToken)
    {
        var organizationId = access.Organization.Id;
        var teams = await readTeams
            .Where(t => t.OrganizationId == organizationId)
            .OrderBy(t => t.Name)
            .ToListAsync(cancellationToken);
        var teamIds = teams.Select(t => t.Id).ToList();
        var members = await readTeamMembers
            .Where(tm => teamIds.Contains(tm.TeamId))
            .Select(tm => new { tm.TeamId, tm.AccountId })
            .ToListAsync(cancellationToken);
        var projectLinks = await readProjectTeams
            .Where(pt => teamIds.Contains(pt.TeamId))
            .Select(pt => pt.TeamId)
            .ToListAsync(cancellationToken);

        return teams.Select(t =>
        {
            var isMember = members.Any(m => m.TeamId == t.Id && m.AccountId == access.Member.AccountId);
            return new TeamSummaryPresentation
            {
                Id = t.Id,
                Name = t.Name,
                MemberCount = members.Count(m => m.TeamId == t.Id),
                ProjectCount = projectLinks.Count(id => id == t.Id),
                IsMember = isMember,
                CanManage = OrgPermissions.CanManageTeam(access.Role, isMember),
            };
        }).ToList();
    }

    public static async Task<TeamPresentation> PresentTeam(
        TeamEntity team,
        OrgAccess access,
        Read<TeamMemberEntity> readTeamMembers,
        Read<AccountEntity> readAccounts,
        Read<OrgMemberEntity> readOrgMembers,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<ProjectEntity> readProjects,
        CancellationToken cancellationToken)
    {
        var members = await readTeamMembers
            .Where(tm => tm.TeamId == team.Id)
            .Join(readAccounts, tm => tm.AccountId, a => a.Id, (tm, a) => a)
            .Join(readOrgMembers.Where(m => m.OrganizationId == team.OrganizationId), a => a.Id, m => m.AccountId,
                (a, m) => new { Account = a, m.Role })
            .OrderBy(x => x.Account.Name)
            .ToListAsync(cancellationToken);
        var projects = await readProjectTeams
            .Where(pt => pt.TeamId == team.Id)
            .Join(readProjects, pt => pt.ProjectId, p => p.Id, (_, p) => p)
            .OrderBy(p => p.Name)
            .ToListAsync(cancellationToken);

        var isMember = members.Any(x => x.Account.Id == access.Member.AccountId);
        return new TeamPresentation
        {
            Id = team.Id,
            OrganizationId = team.OrganizationId,
            Name = team.Name,
            CreatedAt = team.CreatedAt,
            Members = members.Select(x => new TeamMemberPresentation
            {
                AccountId = x.Account.ExternalId,
                Email = x.Account.Login,
                Name = x.Account.Name,
                Role = OrgRoles.ToValue(OrgRoles.Parse(x.Role)),
            }).ToList(),
            Projects = projects.Select(ProjectPresenter.PresentRef).ToList(),
            IsMember = isMember,
            CanManage = OrgPermissions.CanManageTeam(access.Role, isMember),
        };
    }
}
