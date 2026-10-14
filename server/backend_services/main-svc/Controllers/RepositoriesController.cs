using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Presentation;
using core.Rules;
using core.Services.GitHub;
using core.Validations;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Mvc;

namespace main_svc.Controllers;

public class LinkRepositoryRequest
{
    public required int ConnectionId { get; set; }

    /// <summary>`owner/name`, or the repository's GitHub URL.</summary>
    public required string FullName { get; set; }
}

/// <summary>
/// GitHub repositories linked to a project. Linking and syncing read the default branch's envspecs and appspecs
/// right away; monitor-svc re-syncs and checks the deployments from then on.
/// </summary>
[ApiController]
public class RepositoriesController : ControllerBase
{
    [HttpGet("Projects/{projectId:int}/Repositories")]
    public async Task<ActionResult<List<RepositoryPresentation>>> GetRepositories(
        int projectId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<RepositoryEntity> readRepositories,
        [FromServices] Read<DeployEnvironmentEntity> readEnvironments,
        [FromServices] Read<DeploymentEntity> readDeployments
    )
    {
        using var vr = new ValidationResult();
        var found = await ProjectValidation.TryGetProjectAccess(projectId, claims.AccountId, vr, readProjects,
            readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (_, access, isProjectMember) = found.Value;
        return await RepositoryPresenter.PresentRepositories(projectId,
            OrgPermissions.CanManageProject(access.Role, isProjectMember), readRepositories, readEnvironments,
            readDeployments, context.CancellationToken);
    }

    [HttpPost("Projects/{projectId:int}/Repositories")]
    public async Task<ActionResult<RepositoryPresentation>> Link(
        int projectId,
        LinkRepositoryRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] IGitHub github,
        [FromServices] TokenCipher cipher,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<GitHubConnectionEntity> readConnections,
        [FromServices] Mutate<RepositoryEntity> mutateRepositories,
        [FromServices] Mutate<DeployEnvironmentEntity> mutateEnvironments,
        [FromServices] Mutate<DeploymentEntity> mutateDeployments
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
        if (!OrgValidation.EnsureAllowed(OrgPermissions.CanManageProject(access.Role, isProjectMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var fullName = RepositoryValidation.ValidateFullName(request.FullName, vr);
        var connection = await GitHubConnectionValidation.TryGetConnection(project.OrganizationId,
            request.ConnectionId, vr, readConnections);
        if (fullName == null || connection == null)
        {
            return vr.ReturnErrorResponse();
        }

        var token = GitHubConnectionOps.Token(connection, cipher);
        var gitHubRepository = await RepositoryValidation.TryGetGitHubRepository(token, fullName.Value.Owner,
            fullName.Value.Name, vr, github, context.CancellationToken);
        if (gitHubRepository == null ||
            !await RepositoryValidation.EnsureNotLinked(project.Id, gitHubRepository, vr, mutateRepositories.Read))
        {
            return vr.ReturnErrorResponse();
        }

        var now = context.Clock.Now;
        var repository = await RepositoryOps.Create(
            projectId: project.Id,
            connection: connection,
            gitHubRepository: gitHubRepository,
            now: now,
            mutateRepositories: mutateRepositories);
        await RepositoryOps.Sync(
            repository: repository,
            token: token,
            github: github,
            now: now,
            mutateEnvironments: mutateEnvironments,
            mutateDeployments: mutateDeployments,
            cancellationToken: context.CancellationToken);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await RepositoryPresenter.PresentRepository(repository, canManage: true, mutateEnvironments.Read,
            mutateDeployments.Read, context.CancellationToken);
    }

    [HttpGet("Repositories/{repositoryId:int}")]
    public async Task<ActionResult<RepositoryPresentation>> Get(
        int repositoryId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<RepositoryEntity> readRepositories,
        [FromServices] Read<DeployEnvironmentEntity> readEnvironments,
        [FromServices] Read<DeploymentEntity> readDeployments
    )
    {
        using var vr = new ValidationResult();
        var found = await RepositoryValidation.TryGetRepositoryAccess(repositoryId, claims.AccountId, vr,
            readRepositories, readProjects, readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (repository, _, access, isProjectMember) = found.Value;
        return await RepositoryPresenter.PresentRepository(repository,
            OrgPermissions.CanManageProject(access.Role, isProjectMember), readEnvironments, readDeployments,
            context.CancellationToken);
    }

    /// <summary>Re-reads the spec files now, rather than at monitor-svc's next round.</summary>
    [HttpPost("Repositories/{repositoryId:int}/Sync")]
    public async Task<ActionResult<RepositoryPresentation>> Sync(
        int repositoryId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] IGitHub github,
        [FromServices] TokenCipher cipher,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<RepositoryEntity> readRepositories,
        [FromServices] Read<GitHubConnectionEntity> readConnections,
        [FromServices] Mutate<DeployEnvironmentEntity> mutateEnvironments,
        [FromServices] Mutate<DeploymentEntity> mutateDeployments
    )
    {
        using var vr = new ValidationResult();
        var found = await RepositoryValidation.TryGetRepositoryAccess(repositoryId, claims.AccountId, vr,
            readRepositories, readProjects, readMembers, readOrganizations, readProjectTeams, readTeamMembers);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageProject(found.Value.Access.Role, found.Value.IsProjectMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var repository = found.Value.Repository;
        var connection = (await GitHubConnectionOps.TryGet(repository.GitHubConnectionId, readConnections))!;
        await RepositoryOps.Sync(
            repository: repository,
            token: GitHubConnectionOps.Token(connection, cipher),
            github: github,
            now: context.Clock.Now,
            mutateEnvironments: mutateEnvironments,
            mutateDeployments: mutateDeployments,
            cancellationToken: context.CancellationToken);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await RepositoryPresenter.PresentRepository(repository, canManage: true, mutateEnvironments.Read,
            mutateDeployments.Read, context.CancellationToken);
    }

    [HttpDelete("Repositories/{repositoryId:int}")]
    public async Task<ActionResult> Unlink(
        int repositoryId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Mutate<RepositoryEntity> mutateRepositories
    )
    {
        using var vr = new ValidationResult();
        var found = await RepositoryValidation.TryGetRepositoryAccess(repositoryId, claims.AccountId, vr,
            mutateRepositories.Read, readProjects, readMembers, readOrganizations, readProjectTeams,
            readTeamMembers);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageProject(found.Value.Access.Role, found.Value.IsProjectMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        RepositoryOps.Delete(found.Value.Repository, mutateRepositories);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }
}
