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

public class OrganizationRequest
{
    public required string Name { get; set; }
}

public class RoleRequest
{
    public required string Role { get; set; }
}

[ApiController]
[Route("Orgs")]
public class OrgsController : ControllerBase
{
    /// <summary>Another organization, owned by the caller.</summary>
    [HttpPost]
    public async Task<ActionResult<OrganizationPresentation>> Create(
        OrganizationRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<OrganizationEntity> mutateOrganizations,
        [FromServices] Mutate<OrgMemberEntity> mutateMembers
    )
    {
        using var vr = new ValidationResult();
        if (!OrganizationValidation.ValidateName(request.Name, vr))
        {
            return vr.ReturnErrorResponse();
        }

        var account = (await AccountOps.TryGetAccountById(claims.AccountId, readAccounts))!;
        var (organization, owner) = await OrganizationOps.Create(
            name: request.Name,
            owner: account,
            now: context.Clock.Now,
            mutateOrganizations: mutateOrganizations,
            mutateMembers: mutateMembers);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return OrganizationPresenter.PresentOrganization(new OrgAccess(organization, owner));
    }

    /// <summary>The organization with the caller's role and permissions.</summary>
    [HttpGet("{orgId:int}")]
    public async Task<ActionResult<OrganizationPresentation>> Get(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        return OrganizationPresenter.PresentOrganization(access);
    }

    /// <summary>Renames the organization; the slug stays. Owner only.</summary>
    [HttpPut("{orgId:int}")]
    public async Task<ActionResult<OrganizationPresentation>> Update(
        int orgId,
        OrganizationRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanManageOrganization(access.Role), vr) ||
            !OrganizationValidation.ValidateName(request.Name, vr))
        {
            return vr.ReturnErrorResponse();
        }

        access.Organization.Name = request.Name.Trim();

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return OrganizationPresenter.PresentOrganization(access);
    }

    /// <summary>Deletes the organization with its members, invitations, teams and projects. Owner only.</summary>
    [HttpDelete("{orgId:int}")]
    public async Task<ActionResult> Delete(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Mutate<OrganizationEntity> mutateOrganizations
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers,
            mutateOrganizations.Read);
        if (access == null || !OrgValidation.EnsureAllowed(OrgPermissions.CanManageOrganization(access.Role), vr))
        {
            return vr.ReturnErrorResponse();
        }

        OrganizationOps.Delete(access.Organization, mutateOrganizations);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    [HttpGet("{orgId:int}/Members")]
    public async Task<ActionResult<List<MemberPresentation>>> GetMembers(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await OrganizationPresenter.PresentMembers(access, readMembers, readAccounts,
            context.CancellationToken);
    }

    /// <summary>Owners change any role; admins any but owner, and never an owner's. The last owner stays.</summary>
    [HttpPut("{orgId:int}/Members/{accountId:guid}/Role")]
    public async Task<ActionResult> SetRole(
        int orgId,
        Guid accountId,
        RoleRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        var newRole = OrgValidation.ValidateRole(request.Role, vr);
        var target = newRole == null
            ? null
            : await OrgValidation.TryGetMember(orgId, accountId, vr, readAccounts, readMembers);
        if (target == null ||
            !OrgValidation.EnsureAllowed(
                OrgPermissions.CanChangeRole(access.Role, OrgRoles.Parse(target.Value.Member.Role), newRole!.Value),
                vr) ||
            !await OrgValidation.EnsureNotLastOwner(target.Value.Member, newRole, vr, readMembers))
        {
            return vr.ReturnErrorResponse();
        }

        target.Value.Member.Role = OrgRoles.ToValue(newRole.Value);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    /// <summary>Removes a member (owners and admins) or leaves the organization (anyone). The last owner stays.</summary>
    [HttpDelete("{orgId:int}/Members/{accountId:guid}")]
    public async Task<ActionResult> RemoveMember(
        int orgId,
        Guid accountId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Mutate<OrgMemberEntity> mutateMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<TeamMemberEntity> mutateTeamMembers,
        [FromServices] Read<TeamEntity> readTeams
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, mutateMembers.Read,
            readOrganizations);
        var target = access == null
            ? null
            : await OrgValidation.TryGetMember(orgId, accountId, vr, readAccounts, mutateMembers.Read);
        if (target == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanRemoveMember(access!.Role,
                OrgRoles.Parse(target.Value.Member.Role), isSelf: target.Value.Account.Id == claims.AccountId), vr) ||
            !await OrgValidation.EnsureNotLastOwner(target.Value.Member, null, vr, mutateMembers.Read))
        {
            return vr.ReturnErrorResponse();
        }

        await OrgMemberOps.Remove(target.Value.Member, mutateMembers, mutateTeamMembers, readTeams);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }
}
