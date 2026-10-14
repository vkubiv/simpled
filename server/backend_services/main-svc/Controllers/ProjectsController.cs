using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Presentation;
using core.Primitive;
using core.Rules;
using core.Validations;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Mvc;

namespace main_svc.Controllers;

public class CreateProjectRequest
{
    public required string Name { get; set; }

    /// <summary>Made from the name when empty.</summary>
    public string? Slug { get; set; }

    public List<int> TeamIds { get; set; } = [];
}

public class UpdateProjectRequest
{
    public required string Name { get; set; }
    public required string Slug { get; set; }
}

[ApiController]
public class ProjectsController : ControllerBase
{
    [HttpGet("Orgs/{orgId:int}/Projects")]
    public async Task<ActionResult<List<ProjectPresentation>>> GetProjects(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        var myTeamIds = await TeamOps.TeamIdsOf(orgId, claims.AccountId, readTeams, readTeamMembers);
        return await ProjectPresenter.PresentProjects(access, myTeamIds, readProjects, readProjectTeams, readTeams,
            context.CancellationToken);
    }

    /// <summary>Owners, admins and managers; a manager's project links only teams they are in, at least one.</summary>
    [HttpPost("Orgs/{orgId:int}/Projects")]
    public async Task<ActionResult<ProjectPresentation>> Create(
        int orgId,
        CreateProjectRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Mutate<ProjectEntity> mutateProjects,
        [FromServices] Mutate<ProjectTeamEntity> mutateProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanCreateProject(access.Role), vr) ||
            !ProjectValidation.ValidateName(request.Name, vr) ||
            !ProjectValidation.ValidateSlug(request.Slug, vr, required: false) ||
            !await ProjectValidation.EnsureSlugFree(orgId, request.Slug, null, vr, mutateProjects.Read))
        {
            return vr.ReturnErrorResponse();
        }

        var teams = await TeamValidation.TryGetTeams(orgId, request.TeamIds, vr, readTeams);
        var myTeamIds = await TeamOps.TeamIdsOf(orgId, claims.AccountId, readTeams, readTeamMembers);
        if (teams == null ||
            !OrgValidation.EnsureAllowed(
                access.Role != OrgRole.Manager || (teams.Count > 0 && teams.All(t => myTeamIds.Contains(t.Id))), vr,
                "teamIds"))
        {
            return vr.ReturnErrorResponse();
        }

        var project = await ProjectOps.Create(
            organizationId: orgId,
            name: request.Name,
            slug: request.Slug,
            teams: teams,
            now: context.Clock.Now,
            mutateProjects: mutateProjects,
            mutateProjectTeams: mutateProjectTeams);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await ProjectPresenter.PresentProject(project, access, myTeamIds, mutateProjectTeams.Read, readTeams,
            context.CancellationToken);
    }

    [HttpGet("Projects/{projectId:int}")]
    public async Task<ActionResult<ProjectPresentation>> Get(
        int projectId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await ProjectValidation.TryGetProjectAccess(projectId, claims.AccountId, vr, readProjects,
            readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (project, access, _) = found.Value;
        var myTeamIds = await TeamOps.TeamIdsOf(project.OrganizationId, claims.AccountId, readTeams,
            readTeamMembers);
        return await ProjectPresenter.PresentProject(project, access, myTeamIds, readProjectTeams, readTeams,
            context.CancellationToken);
    }

    [HttpPut("Projects/{projectId:int}")]
    public async Task<ActionResult<ProjectPresentation>> Update(
        int projectId,
        UpdateProjectRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await ProjectValidation.TryGetProjectAccess(projectId, claims.AccountId, vr, readProjects,
            readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (project, access, isProjectMember) = found.Value;
        if (!OrgValidation.EnsureAllowed(OrgPermissions.CanManageProject(access.Role, isProjectMember), vr) ||
            !ProjectValidation.ValidateName(request.Name, vr) ||
            !ProjectValidation.ValidateSlug(request.Slug, vr, required: true) ||
            !await ProjectValidation.EnsureSlugFree(project.OrganizationId, request.Slug, project.Id, vr,
                readProjects))
        {
            return vr.ReturnErrorResponse();
        }

        project.Name = request.Name.Trim();
        project.Slug = request.Slug.Trim();

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        var myTeamIds = await TeamOps.TeamIdsOf(project.OrganizationId, claims.AccountId, readTeams,
            readTeamMembers);
        return await ProjectPresenter.PresentProject(project, access, myTeamIds, readProjectTeams, readTeams,
            context.CancellationToken);
    }

    [HttpDelete("Projects/{projectId:int}")]
    public async Task<ActionResult> Delete(
        int projectId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Mutate<ProjectEntity> mutateProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await ProjectValidation.TryGetProjectAccess(projectId, claims.AccountId, vr,
            mutateProjects.Read, readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageProject(found.Value.Access.Role, found.Value.IsProjectMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        ProjectOps.Delete(found.Value.Project, mutateProjects);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    /// <summary>Links a team of the same organization; a manager needs both the project and the team.</summary>
    [HttpPost("Projects/{projectId:int}/Teams/{teamId:int}")]
    public async Task<ActionResult> LinkTeam(
        int projectId,
        int teamId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Mutate<ProjectTeamEntity> mutateProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var linkable = await TryGetLinkable(projectId, teamId, claims.AccountId, vr, readProjects, readMembers,
            readOrganizations, mutateProjectTeams.Read, readTeams, readTeamMembers);
        if (linkable == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (project, team) = linkable.Value;
        if (await ProjectOps.TryGetLink(project.Id, team.Id, mutateProjectTeams.Read) != null)
        {
            vr.AlreadyExists("teamId", "The team is already linked.");
            return vr.ReturnErrorResponse();
        }

        await ProjectOps.LinkTeam(project, team, mutateProjectTeams);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    [HttpDelete("Projects/{projectId:int}/Teams/{teamId:int}")]
    public async Task<ActionResult> UnlinkTeam(
        int projectId,
        int teamId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Mutate<ProjectTeamEntity> mutateProjectTeams,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var linkable = await TryGetLinkable(projectId, teamId, claims.AccountId, vr, readProjects, readMembers,
            readOrganizations, mutateProjectTeams.Read, readTeams, readTeamMembers);
        if (linkable == null)
        {
            return vr.ReturnErrorResponse();
        }

        var link = await ProjectOps.TryGetLink(projectId, teamId, mutateProjectTeams.Read);
        if (link == null)
        {
            vr.DoesntExist("teamId", "The team is not linked.");
            return vr.ReturnErrorResponse();
        }

        mutateProjectTeams.Remove(link);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    /// <summary>The project and a team of its organization, when the caller may manage both.</summary>
    private static async Task<(ProjectEntity Project, TeamEntity Team)?> TryGetLinkable(
        int projectId,
        int teamId,
        int accountId,
        ValidationResult vr,
        Read<ProjectEntity> readProjects,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamEntity> readTeams,
        Read<TeamMemberEntity> readTeamMembers)
    {
        var found = await ProjectValidation.TryGetProjectAccess(projectId, accountId, vr, readProjects,
            readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null)
        {
            return null;
        }

        var (project, access, isProjectMember) = found.Value;
        var teams = await TeamValidation.TryGetTeams(project.OrganizationId, [teamId], vr, readTeams, "teamId");
        if (teams == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanManageProject(access.Role, isProjectMember) &&
                OrgPermissions.CanManageTeam(access.Role,
                    await TeamOps.IsMember(teamId, accountId, readTeamMembers)), vr))
        {
            return null;
        }

        return (project, teams[0]);
    }
}
