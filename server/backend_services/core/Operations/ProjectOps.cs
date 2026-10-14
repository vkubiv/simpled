using core.Libs.EntityAccess;
using core.Primitive;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class ProjectOps
{
    public static Task<ProjectEntity?> TryGet(int projectId, Read<ProjectEntity> readProjects) =>
        readProjects.FirstOrDefaultAsync(p => p.Id == projectId);

    /// <summary>
    /// Creates the project linked to <paramref name="teams"/>. Without a <paramref name="slug"/> one is made from
    /// the name, unique within the organization; a given slug is validated by the caller.
    /// </summary>
    public static async Task<ProjectEntity> Create(
        int organizationId,
        string name,
        string? slug,
        IEnumerable<TeamEntity> teams,
        DateTime now,
        Mutate<ProjectEntity> mutateProjects,
        Mutate<ProjectTeamEntity> mutateProjectTeams)
    {
        var project = new ProjectEntity
        {
            OrganizationId = organizationId,
            Name = name.Trim(),
            Slug = string.IsNullOrWhiteSpace(slug)
                ? await UniqueSlug(organizationId, name, mutateProjects.Read)
                : slug.Trim(),
            CreatedAt = now,
        };
        await mutateProjects.AddAsync(project);

        foreach (var team in teams)
        {
            await LinkTeam(project, team, mutateProjectTeams);
        }

        return project;
    }

    /// <summary>Removes the project; the database cascades to its team links.</summary>
    public static void Delete(ProjectEntity project, Mutate<ProjectEntity> mutateProjects) =>
        mutateProjects.Remove(project);

    /// <summary>True when the account is in a team linked to the project.</summary>
    public static Task<bool> IsMember(
        int projectId,
        int accountId,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamMemberEntity> readTeamMembers) =>
        readProjectTeams
            .Where(pt => pt.ProjectId == projectId)
            .Join(readTeamMembers.Where(tm => tm.AccountId == accountId), pt => pt.TeamId, tm => tm.TeamId,
                (pt, _) => pt.Id)
            .AnyAsync();

    public static Task<ProjectTeamEntity?> TryGetLink(int projectId, int teamId,
        Read<ProjectTeamEntity> readProjectTeams) =>
        readProjectTeams.FirstOrDefaultAsync(pt => pt.ProjectId == projectId && pt.TeamId == teamId);

    public static async Task<ProjectTeamEntity> LinkTeam(
        ProjectEntity project,
        TeamEntity team,
        Mutate<ProjectTeamEntity> mutateProjectTeams)
    {
        var link = new ProjectTeamEntity { Project = project, Team = team };
        await mutateProjectTeams.AddAsync(link);
        return link;
    }

    public static Task<bool> IsSlugTaken(int organizationId, string slug, int? exceptProjectId,
        Read<ProjectEntity> readProjects) =>
        readProjects.AnyAsync(p => p.OrganizationId == organizationId && p.Slug == slug && p.Id != exceptProjectId);

    private static async Task<string> UniqueSlug(int organizationId, string name, Read<ProjectEntity> readProjects)
    {
        var slug = Slug.From(name, "project");
        var taken = await readProjects
            .Where(p => p.OrganizationId == organizationId && (p.Slug == slug || p.Slug.StartsWith(slug + "-")))
            .Select(p => p.Slug)
            .ToListAsync();
        return Slug.FirstFree(slug, taken);
    }
}
