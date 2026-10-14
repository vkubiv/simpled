using core.Auth;
using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Presentation;
using core.Primitive;
using core.Rules;
using core.Services.Mailer;
using core.Validations;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Authorization;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.RateLimiting;

namespace main_svc.Controllers;

public class InviteRequest
{
    public required string Email { get; set; }
    public required string Role { get; set; }
}

public class InvitationRegisterRequest
{
    public required string Token { get; set; }
    public required string Name { get; set; }
    public required string Password { get; set; }
}

/// <summary>
/// Invitations are managed by owners and admins inside an organization. The invited person opens the mailed
/// link (web app page <c>invitation?token=</c>); tokens travel in request bodies so they stay out of access logs.
/// </summary>
[ApiController]
public class InvitationsController : ControllerBase
{
    /// <summary>Mails an invitation; a pending one to the same email is replaced.</summary>
    [HttpPost("Orgs/{orgId:int}/Invitations")]
    public async Task<ActionResult<InvitationPresentation>> Invite(
        int orgId,
        InviteRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<OrgInvitationEntity> mutateInvitations,
        [FromServices] UserMailer userMailer
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null || !OrgValidation.EnsureAllowed(OrgPermissions.CanManageMembers(access.Role), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var role = OrgValidation.ValidateRole(request.Role, vr);
        if (!AccountValidation.ValidateEmail(request.Email, vr) || role == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanAssign(access.Role, role.Value), vr) ||
            !await InvitationValidation.EnsureNotMember(orgId, request.Email, vr, readAccounts, readMembers))
        {
            return vr.ReturnErrorResponse();
        }

        var now = context.Clock.Now;
        var inviter = (await AccountOps.TryGetAccountById(claims.AccountId, readAccounts))!;
        var (token, invitation) = await OrgInvitationOps.Create(
            organization: access.Organization,
            email: request.Email,
            role: role.Value,
            invitedBy: inviter,
            now: now,
            mutateInvitations: mutateInvitations);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        await userMailer.SendOrgInvitation(invitation.Email, access.Organization.Name, inviter.Name,
            invitation.Role, token);
        return InvitationPresenter.PresentInvitation(invitation, inviter.Name, now);
    }

    /// <summary>Open invitations (expired ones included, for a resend). Owners and admins.</summary>
    [HttpGet("Orgs/{orgId:int}/Invitations")]
    public async Task<ActionResult<List<InvitationPresentation>>> GetInvitations(
        int orgId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<OrgInvitationEntity> readInvitations
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null || !OrgValidation.EnsureAllowed(OrgPermissions.CanManageMembers(access.Role), vr))
        {
            return vr.ReturnErrorResponse();
        }

        return await InvitationPresenter.PresentOpen(orgId, context.Clock.Now, readInvitations, readAccounts,
            context.CancellationToken);
    }

    /// <summary>Revokes an open invitation; admins can't revoke an invitation as owner.</summary>
    [HttpDelete("Orgs/{orgId:int}/Invitations/{invitationId:int}")]
    public async Task<ActionResult> Revoke(
        int orgId,
        int invitationId,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<OrgInvitationEntity> readInvitations
    )
    {
        using var vr = new ValidationResult();
        var access = await OrgValidation.TryGetAccess(orgId, claims.AccountId, vr, readMembers, readOrganizations);
        if (access == null || !OrgValidation.EnsureAllowed(OrgPermissions.CanManageMembers(access.Role), vr))
        {
            return vr.ReturnErrorResponse();
        }

        var invitation = await InvitationValidation.TryGetOpen(orgId, invitationId, vr, readInvitations);
        if (invitation == null ||
            !OrgValidation.EnsureAllowed(OrgPermissions.CanAssign(access.Role, OrgRoles.Parse(invitation.Role)), vr))
        {
            return vr.ReturnErrorResponse();
        }

        invitation.RevokedAt = context.Clock.Now;

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);
        return Ok();
    }

    /// <summary>What the invitation page shows: organization, role, and whether to sign in or register.</summary>
    [AllowAnonymous]
    [EnableRateLimiting(AuthRateLimiting.Policy)]
    [HttpPost("Invitations/Preview")]
    public async Task<ActionResult<InvitationPreviewPresentation>> Preview(
        TokenRequest request,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgInvitationEntity> readInvitations,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts
    )
    {
        using var vr = new ValidationResult();
        var invitation = await InvitationValidation.TryGetLive(request.Token, context.Clock.Now, vr, readInvitations);
        if (invitation == null)
        {
            return vr.ReturnErrorResponse();
        }

        return await InvitationPresenter.PresentPreview(invitation, readOrganizations, readAccounts,
            context.CancellationToken);
    }

    /// <summary>Joins the signed-in account, whose email must be the invited one.</summary>
    [HttpPost("Invitations/Accept")]
    public async Task<ActionResult<MyOrganizationPresentation>> Accept(
        TokenRequest request,
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<OrgInvitationEntity> readInvitations,
        [FromServices] Read<OrganizationEntity> readOrganizations,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Mutate<OrgMemberEntity> mutateMembers
    )
    {
        var now = context.Clock.Now;

        using var vr = new ValidationResult();
        var invitation = await InvitationValidation.TryGetLive(request.Token, now, vr, readInvitations);
        if (invitation == null)
        {
            return vr.ReturnErrorResponse();
        }

        var account = (await AccountOps.TryGetAccountById(claims.AccountId, readAccounts))!;
        if (account.Login != invitation.Email)
        {
            vr.IncorrectState("token", $"This invitation is for {invitation.Email}; sign in with that email.");
            return vr.ReturnErrorResponse();
        }

        if (await OrgMemberOps.TryGet(invitation.OrganizationId, account.Id, mutateMembers.Read) != null)
        {
            vr.AlreadyExists("token", "You are already a member of this organization.");
            return vr.ReturnErrorResponse();
        }

        var member = await OrgInvitationOps.Accept(invitation, account, now, mutateMembers);

        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        var organization = await readOrganizations.FindAsync(invitation.OrganizationId);
        return MePresenter.PresentMyOrganization(organization!, member);
    }

    /// <summary>
    /// Creates the invited person's account, confirmed (the link proves the email), joins the organization and
    /// signs in.
    /// </summary>
    [AllowAnonymous]
    [EnableRateLimiting(AuthRateLimiting.Policy)]
    [HttpPost("Invitations/Register")]
    public async Task<ActionResult<SessionTokenResponse>> Register(
        InvitationRegisterRequest request,
        [FromQuery(Name = "useToken")] bool useToken,
        [FromServices] MainCallContext context,
        [FromServices] SessionCookie sessionCookie,
        [FromServices] Read<OrgInvitationEntity> readInvitations,
        [FromServices] Mutate<AccountEntity> mutateAccounts,
        [FromServices] Mutate<OrgMemberEntity> mutateMembers,
        [FromServices] Mutate<AuthSessionEntity> mutateSessions
    )
    {
        var now = context.Clock.Now;

        using var vr = new ValidationResult();
        var invitation = await InvitationValidation.TryGetLive(request.Token, now, vr, readInvitations);
        if (invitation == null ||
            !await AccountValidation.EnsureAccountNotExist(invitation.Email, vr, mutateAccounts.Read) ||
            !AccountValidation.ValidateName(request.Name, vr) ||
            !AccountValidation.ValidatePassword(request.Password, invitation.Email, vr))
        {
            return vr.ReturnErrorResponse();
        }

        var account = await AccountOps.CreateAccount(
            login: invitation.Email,
            name: request.Name,
            password: request.Password,
            confirmed: true,
            now: now,
            mutateAccounts: mutateAccounts);
        await OrgInvitationOps.Accept(invitation, account, now, mutateMembers);
        // The session row needs the account's id.
        await context.UnitOfWork.SaveChangesAsync(context.CancellationToken);

        return await SessionSignIn.SignIn(this, account, useToken, context, sessionCookie, mutateSessions);
    }
}
