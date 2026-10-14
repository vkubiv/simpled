using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Presentation;
using core.Validations;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Mvc;

namespace main_svc.Controllers;

[ApiController]
public class DeploymentsController : ControllerBase
{
    /// <summary>What the monitor noticed about the deployment, newest first, at most 50.</summary>
    [HttpGet("Deployments/{deploymentId:int}/Events")]
    public async Task<ActionResult<List<DeploymentEventPresentation>>> GetEvents(
        int deploymentId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<ProjectEntity> readProjects,
        [FromServices] Read<RepositoryEntity> readRepositories,
        [FromServices] Read<DeployEnvironmentEntity> readEnvironments,
        [FromServices] Read<DeploymentEntity> readDeployments,
        [FromServices] Read<DeploymentEventEntity> readEvents
    )
    {
        using var vr = new ValidationResult();
        var found = await DeploymentValidation.TryGetDeploymentAccess(deploymentId, claims.AccountId, vr,
            readDeployments, readEnvironments, readRepositories, readProjects, readMembers, readOrganizations);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await RepositoryPresenter.PresentEvents(deploymentId, readEvents, context.CancellationToken);
    }
}
