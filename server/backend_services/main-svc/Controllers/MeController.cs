using core.Libs.EntityAccess;
using core.Operations;
using core.Presentation;
using db.Entities;
using main_svc.Auth;
using Microsoft.AspNetCore.Mvc;

namespace main_svc.Controllers;

[ApiController]
[Route("[controller]")]
public class MeController : ControllerBase
{
    /// <summary>The signed-in account and the organizations it belongs to, with its role in each.</summary>
    [HttpGet]
    public async Task<ActionResult<MePresentation>> Get(
        [FromServices] MainClaims claims,
        [FromServices] MainCallContext context,
        [FromServices] Read<AccountEntity> readAccounts,
        [FromServices] Read<OrgMemberEntity> readMembers,
        [FromServices] Read<OrganizationEntity> readOrganizations
    )
    {
        var account = await AccountOps.TryGetAccountById(claims.AccountId, readAccounts);
        if (account == null)
        {
            return Unauthorized();
        }

        return await MePresenter.PresentMe(account, readMembers, readOrganizations, context.CancellationToken);
    }
}
