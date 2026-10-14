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

public class TeamRequest
{
    public required string Name { get; set; }
}

[ApiController]
public class TeamsController : ControllerBase
{
    [HttpGet("Orgs/{orgId:int}/Teams")]
    public async Task<ActionResult<List<TeamSummaryPresentation>>> GetTeams(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await TeamPresenter.PresentTeams(access, readTeams, readTeamMembers, readProjectTeams,
            context.CancellationToken);
    }

    /// <summary>Owners, admins and managers; a manager joins the team they create, so they can manage it.</summary>
    [HttpPost("Orgs/{orgId:int}/Teams")]
    public async Task<ActionResult<TeamPresentation>> Create(
        int orgId,
        TeamRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<TeamEntity> mutateTeams,
        [FromServices] Mutate<TeamMemberEntity> mutateTeamMembers,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<ProjectEntity> readProjects
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanCreateTeam(access.Role), vr) ||
            !TeamValidation.ValidateName(request.Name, vr) ||
            !await TeamValidation.EnsureNameFree(orgId, request.Name, null, vr, mutateTeams.Read))
        {
            return vr.ReturnErrorResponse();
        }

        var now = context.Clock.Now;
        var team = await TeamOps.Create(orgId, request.Name, now, mutateTeams);
        if (access.Role == OrgRole.Manager)
        {
            var account = (await AccountOps.TryGetAccountById(claims.AccountId, readAccounts))!;
            await TeamOps.AddMember(team, account, now, mutateTeamMembers);
        }

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await TeamPresenter.PresentTeam(team, access, mutateTeamMembers.Read, readAccounts, readMembers,
            readProjectTeams, readProjects, context.CancellationToken);
    }

    [HttpGet("Teams/{teamId:int}")]
    public async Task<ActionResult<TeamPresentation>> Get(
        int teamId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<ProjectEntity> readProjects
    )
    {
        using var vr = new ValidationResult();
        var found = await TeamValidation.TryGetTeamAccess(teamId, claims.AccountId, vr, readTeams, readMembers,
            readOrganizations, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await TeamPresenter.PresentTeam(found.Value.Team, found.Value.Access, readTeamMembers, readAccounts,
            readMembers, readProjectTeams, readProjects, context.CancellationToken);
    }

    [HttpPut("Teams/{teamId:int}")]
    public async Task<ActionResult<TeamPresentation>> Update(
        int teamId,
        TeamRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers,
        [FromServices] Read<ProjectTeamEntity> readProjectTeams,
        [FromServices] Read<ProjectEntity> readProjects
    )
    {
        using var vr = new ValidationResult();
        var found = await TeamValidation.TryGetTeamAccess(teamId, claims.AccountId, vr, readTeams, readMembers,
            readOrganizations, readTeamMembers);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (team, access, isTeamMember) = found.Value;
        if (!OrgValidation.EnsureAllowed(OrgPermissions.CanManageTeam(access.Role, isTeamMember), vr) ||
            !TeamValidation.ValidateName(request.Name, vr) ||
            !await TeamValidation.EnsureNameFree(team.OrganizationId, request.Name, team.Id, vr, readTeams))
        {
            return vr.ReturnErrorResponse();
        }

        team.Name = request.Name.Trim();

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await TeamPresenter.PresentTeam(team, access, readTeamMembers, readAccounts, readMembers,
            readProjectTeams, readProjects, context.CancellationToken);
    }

    /// <summary>Deletes the team; its projects stay, unlinked from it.</summary>
    [HttpDelete("Teams/{teamId:int}")]
    public async Task<ActionResult> Delete(
        int teamId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Mutate<TeamEntity> mutateTeams,
        [FromServices] Read<TeamMemberEntity> readTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await TeamValidation.TryGetTeamAccess(teamId, claims.AccountId, vr, mutateTeams.Read,
            readMembers, readOrganizations, readTeamMembers);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageTeam(found.Value.Access.Role, found.Value.IsTeamMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        TeamOps.Delete(found.Value.Team, mutateTeams);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    /// <summary>Adds an organization member to the team.</summary>
    [HttpPost("Teams/{teamId:int}/Members/{accountId:guid}")]
    public async Task<ActionResult> AddMember(
        int teamId,
        Guid accountId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Mutate<TeamMemberEntity> mutateTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await TeamValidation.TryGetTeamAccess(teamId, claims.AccountId, vr, readTeams, readMembers,
            readOrganizations, mutateTeamMembers.Read);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageTeam(found.Value.Access.Role, found.Value.IsTeamMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var team = found.Value.Team;
        var target = await OrgValidation.TryGetMember(team.OrganizationId, accountId, vr, readAccounts, readMembers);
        if (target == null)
        {
            return vr.ReturnErrorResponse();
        }

        if (await TeamOps.IsMember(team.Id, target.Value.Account.Id, mutateTeamMembers.Read))
        {
            vr.AlreadyExists("accountId", "Already in the team.");
            return vr.ReturnErrorResponse();
        }

        await TeamOps.AddMember(team, target.Value.Account, context.Clock.Now, mutateTeamMembers);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    [HttpDelete("Teams/{teamId:int}/Members/{accountId:guid}")]
    public async Task<ActionResult> RemoveMember(
        int teamId,
        Guid accountId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<TeamEntity> readTeams,
        [FromServices] Mutate<TeamMemberEntity> mutateTeamMembers
    )
    {
        using var vr = new ValidationResult();
        var found = await TeamValidation.TryGetTeamAccess(teamId, claims.AccountId, vr, readTeams, readMembers,
            readOrganizations, mutateTeamMembers.Read);
        if (found == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanManageTeam(found.Value.Access.Role, found.Value.IsTeamMember), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var account = await AccountOps.TryGetAccountByExternalId(accountId, readAccounts);
        var teamMember = account == null
            ? null
            : await TeamOps.TryGetMember(teamId, account.Id, mutateTeamMembers.Read);
        if (teamMember == null)
        {
            vr.DoesntExist("accountId", "Not in the team.");
            return vr.ReturnErrorResponse();
        }

        mutateTeamMembers.Remove(teamMember);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }
}
