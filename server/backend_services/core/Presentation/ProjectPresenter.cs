using core.Libs.EntityAccess;
using core.Rules;
using core.Validations;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

public class TeamRefPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
}

public class ProjectPresentation
{
    public required int Id { get; set; }
    public required int OrganizationId { get; set; }
    public required string Name { get; set; }
    public required string Slug { get; set; }
    public required DateTime CreatedAt { get; set; }
    public required List<TeamRefPresentation> Teams { get; set; }

    /// <summary>The caller is in one of the project's teams.</summary>
    public required bool IsMember { get; set; }

    public required bool CanManage { get; set; }
}

public static class ProjectPresenter
{
    public static ProjectRefPresentation PresentRef(ProjectEntity project) => new()
    {
        Id = project.Id,
        Name = project.Name,
        Slug = project.Slug,
    };

    /// <param name="myTeamIds">The caller's teams in the organization (TeamOps.TeamIdsOf).</param>
    public static async Task<List<ProjectPresentation>> PresentProjects(
        OrgAccess access,
        IReadOnlyCollection<int> myTeamIds,
        Read<ProjectEntity> readProjects,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamEntity> readTeams,
        CancellationToken cancellationToken)
    {
        var organizationId = access.Organization.Id;
        var projects = await readProjects
            .Where(p => p.OrganizationId == organizationId)
            .OrderBy(p => p.Name)
            .ToListAsync(cancellationToken);
        var links = await LinkedTeams(projects.Select(p => p.Id).ToList(), readProjectTeams, readTeams,
            cancellationToken);
        return projects.Select(p => Present(p, access, myTeamIds, links)).ToList();
    }

    public static async Task<ProjectPresentation> PresentProject(
        ProjectEntity project,
        OrgAccess access,
        IReadOnlyCollection<int> myTeamIds,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamEntity> readTeams,
        CancellationToken cancellationToken)
    {
        var links = await LinkedTeams([project.Id], readProjectTeams, readTeams, cancellationToken);
        return Present(project, access, myTeamIds, links);
    }

    private static ProjectPresentation Present(ProjectEntity project, OrgAccess access,
        IReadOnlyCollection<int> myTeamIds, IEnumerable<(int ProjectId, TeamEntity Team)> links)
    {
        var teams = links.Where(l => l.ProjectId == project.Id).Select(l => l.Team).ToList();
        var isMember = teams.Any(t => myTeamIds.Contains(t.Id));
        return new ProjectPresentation
        {
            Id = project.Id,
            OrganizationId = project.OrganizationId,
            Name = project.Name,
            Slug = project.Slug,
            CreatedAt = project.CreatedAt,
            Teams = teams.Select(t => new TeamRefPresentation { Id = t.Id, Name = t.Name }).ToList(),
            IsMember = isMember,
            CanManage = OrgPermissions.CanManageProject(access.Role, isMember),
        };
    }

    private static async Task<List<(int ProjectId, TeamEntity Team)>> LinkedTeams(
        List<int> projectIds,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamEntity> readTeams,
        CancellationToken cancellationToken)
    {
        var links = await readProjectTeams
            .Where(pt => projectIds.Contains(pt.ProjectId))
            .Join(readTeams, pt => pt.TeamId, t => t.Id, (pt, t) => new { pt.ProjectId, Team = t })
            .OrderBy(x => x.Team.Name)
            .ToListAsync(cancellationToken);
        return links.Select(x => (x.ProjectId, x.Team)).ToList();
    }
}
