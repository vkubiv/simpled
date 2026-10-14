using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Validations;

public static class DeploymentValidation
{
    /// <summary>The deployment and the caller's access to its organization; others answer <c>doesnt-exist</c>.</summary>
    public static async Task<(DeploymentEntity Deployment, OrgAccess Access)?> TryGetDeploymentAccess(
        int deploymentId,
        int accountId,
        ValidationResult vr,
        Read<DeploymentEntity> readDeployments,
        Read<DeployEnvironmentEntity> readEnvironments,
        Read<RepositoryEntity> readRepositories,
        Read<ProjectEntity> readProjects,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations)
    {
        var found = await readDeployments
            .Where(d => d.Id == deploymentId)
            .Join(readEnvironments, d => d.EnvironmentId, e => e.Id, (d, e) => new { Deployment = d, e.RepositoryId })
            .Join(readRepositories, x => x.RepositoryId, r => r.Id, (x, r) => new { x.Deployment, r.ProjectId })
            .Join(readProjects, x => x.ProjectId, p => p.Id, (x, p) => new { x.Deployment, p.OrganizationId })
            .FirstOrDefaultAsync();
        if (found == null)
        {
            vr.DoesntExist("deploymentId", "Not found.");
            return null;
        }

        var access = await OrgValidation.TryGetAccess(found.OrganizationId, accountId, vr, readMembers,
            readOrganizations, "deploymentId");
        return access == null ? null : (found.Deployment, access);
    }
}
