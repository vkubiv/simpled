using System.Security.Claims;
using Microsoft.AspNetCore.Http;
using Microsoft.AspNetCore.Mvc;

namespace utest.TestSupport;

public static class ControllerHelper
{
    public static void InitContextWithClaimsPrincipal(ControllerBase controller, ClaimsPrincipal claimsPrincipal)
    {
        var controllerContext = controller.ControllerContext;
        controllerContext.HttpContext = new DefaultHttpContext();
        controllerContext.HttpContext.User = claimsPrincipal;
    }
}