using core.Libs.MVCValidation;
using Microsoft.AspNetCore.Mvc;
using Microsoft.AspNetCore.Mvc.Infrastructure;

namespace utest.TestSupport;

public static class ControllerAssert
{
    public static void IsOkResult(IActionResult? result)
    {
        Assert.NotNull(result);
        var statusCodeResult = result as IStatusCodeActionResult;
        Assert.NotNull(statusCodeResult);
        Assert.Equal(200, statusCodeResult.StatusCode);
    }
    
    // 
    public static void IsOkResult<T>(ActionResult<T> result)
    {
        Assert.Null(result.Result);
        Assert.NotNull(result.Value);
        
    }

    public static void IsErrorResult(IActionResult? result)
    {
        Assert.NotNull(result);
        var statusCodeResult = result as IStatusCodeActionResult;
        Assert.NotNull(statusCodeResult);
        Assert.NotEqual(200, statusCodeResult.StatusCode);
    }

    public static void HasValidationError(ActionResult? result, String? withCode = null, String? withValue = null)
    {
        Assert.NotNull(result);
        var statusCodeResult = result as BadRequestObjectResult;
        Assert.NotNull(statusCodeResult);
        Assert.NotEqual(200, statusCodeResult.StatusCode);

        var validationErrors = statusCodeResult.Value as List<ValidationError>;
        Assert.NotNull(validationErrors);
        
        Assert.Contains(validationErrors, e =>
            (withCode == null || e.Code == withCode) && (withValue == null || e.IncorrectValue == withValue)
        );
    }
}