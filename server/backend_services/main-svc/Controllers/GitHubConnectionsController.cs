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

public class CreateGitHubConnectionRequest
{
    public required string Name { get; set; }
    public required string Token { get; set; }
}

public class UpdateGitHubConnectionRequest
{
    public required string Name { get; set; }

    /// <summary>A new token; empty keeps the current one.</summary>
    public string? Token { get; set; }
}

/// <summary>An organization's GitHub personal access tokens. Members see them (without tokens); owners and admins manage them.</summary>
[ApiController]
public class GitHubConnectionsController : ControllerBase
{
    [HttpGet("Orgs/{orgId:int}/GitHubConnections")]
    public async Task<ActionResult<List<GitHubConnectionPresentation>>> GetConnections(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<GitHubConnectionEntity> readConnections,
        [FromServices] Read<RepositoryEntity> readRepositories
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await GitHubConnectionPresenter.PresentConnections(orgId, readConnections, readRepositories,
            context.CancellationToken);
    }

    /// <summary>Checks the token with GitHub and keeps the account it belongs to.</summary>
    [HttpPost("Orgs/{orgId:int}/GitHubConnections")]
    public async Task<ActionResult<GitHubConnectionPresentation>> Create(
        int orgId,
        CreateGitHubConnectionRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] IGitHub github,
        [FromServices] TokenCipher cipher,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Mutate<GitHubConnectionEntity> mutateConnections,
        [FromServices] Read<RepositoryEntity> readRepositories
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanManageGitHubConnections(access.Role), vr) ||
            !GitHubConnectionValidation.ValidateName(request.Name, vr) ||
            !GitHubConnectionValidation.ValidateToken(request.Token, vr) ||
            !await GitHubConnectionValidation.EnsureNameFree(orgId, request.Name, null, vr, mutateConnections.Read))
        {
            return vr.ReturnErrorResponse();
        }

        var user = await GitHubConnectionValidation.TryGetTokenUser(request.Token, vr, github,
            context.CancellationToken);
        if (user == null)
        {
            return vr.ReturnErrorResponse();
        }

        var connection = await GitHubConnectionOps.Create(
            organizationId: orgId,
            name: request.Name,
            token: request.Token,
            user: user,
            now: context.Clock.Now,
            cipher: cipher,
            mutateConnections: mutateConnections);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await GitHubConnectionPresenter.PresentConnection(connection, readRepositories,
            context.CancellationToken);
    }

    [HttpPut("GitHubConnections/{connectionId:int}")]
    public async Task<ActionResult<GitHubConnectionPresentation>> Update(
        int connectionId,
        UpdateGitHubConnectionRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] IGitHub github,
        [FromServices] TokenCipher cipher,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<GitHubConnectionEntity> readConnections,
        [FromServices] Read<RepositoryEntity> readRepositories
    )
    {
        using var vr = new ValidationResult();
        var found = await GitHubConnectionValidation.TryGetConnectionAccess(connectionId, claims.AccountId, vr,
            readConnections, readMembers, readOrganizations);
        if (found == null)
        {
            return vr.ReturnErrorResponse();
        }

        var (connection, access) = found.Value;
        var replacesToken = !string.IsNullOrWhiteSpace(request.Token);
        if (!OrgValidation.EnsureAllowed(OrgPermissions.CanManageGitHubConnections(access.Role), vr) ||
            !GitHubConnectionValidation.ValidateName(request.Name, vr) ||
            (replacesToken && !GitHubConnectionValidation.ValidateToken(request.Token, vr)) ||
            !await GitHubConnectionValidation.EnsureNameFree(connection.OrganizationId, request.Name, connection.Id,
                vr, readConnections))
        {
            return vr.ReturnErrorResponse();
        }

        if (replacesToken)
        {
            var user = await GitHubConnectionValidation.TryGetTokenUser(request.Token!, vr, github,
                context.CancellationToken);
            if (user == null)
            {
                return vr.ReturnErrorResponse();
            }

            GitHubConnectionOps.ReplaceToken(connection, request.Token!, user, cipher);
        }

        connection.Name = request.Name.Trim();

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return await GitHubConnectionPresenter.PresentConnection(connection, readRepositories,
            context.CancellationToken);
    }

    /// <summary>Only a connection no repository uses; unlink them first.</summary>
    [HttpDelete("GitHubConnections/{connectionId:int}")]
    public async Task<ActionResult> Delete(
        int connectionId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Mutate<GitHubConnectionEntity> mutateConnections,
        [FromServices] Read<RepositoryEntity> readRepositories
    )
    {
        using var vr = new ValidationResult();
        var found = await GitHubConnectionValidation.TryGetConnectionAccess(connectionId, claims.AccountId, vr,
            mutateConnections.Read, readMembers, readOrganizations);
        if (found == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanManageGitHubConnections(found.Value.Access.Role), vr) ||
            !await GitHubConnectionValidation.EnsureUnused(found.Value.Connection, vr, readRepositories))
        {
            return vr.ReturnErrorResponse();
        }

        mutateConnections.Remove(found.Value.Connection);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }
}
