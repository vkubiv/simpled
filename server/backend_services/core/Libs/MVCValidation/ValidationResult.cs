using Microsoft.AspNetCore.Mvc;

namespace core.Libs.MVCValidation;

public static class ErrorCode
{
    public const string ValueRequired = "value-required";
    public const string IncorrectValue = "incorrect-value";
    public const string IncorrectState = "incorrect-state";
    public const string AlreadyExist = "already-exist";
    public const string DoesntExist = "doesnt-exist";
    public const string Forbidden = "forbidden";
}

public class ValidationError
{
    public required string Code { get; set; }
    public required string IncorrectValue { get; set; }
    public required string Details { get; set; }
}

public sealed class ValidationResult : IDisposable
{
    public void ValidationError(string code, string incorrectValue, string details)
    {
        _errors.Add(new ValidationError { Code = code, IncorrectValue = incorrectValue, Details = details });
    }

    public void ValueRequired(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.ValueRequired, IncorrectValue = incorrectValue, Details = details });
    }

    public void IncorrectValue(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.IncorrectValue, IncorrectValue = incorrectValue, Details = details });
    }

    public void IncorrectState(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.IncorrectState, IncorrectValue = incorrectValue, Details = details });
    }
    
    public void AlreadyExists(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.AlreadyExist, IncorrectValue = incorrectValue, Details = details });
    }

    public void DoesntExist(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.DoesntExist, IncorrectValue = incorrectValue, Details = details });
    }

    public void Forbidden(string incorrectValue, string details)
    {
        _errors.Add(new ValidationError
            { Code = ErrorCode.Forbidden, IncorrectValue = incorrectValue, Details = details });
    }

    public void Unauthorized()
    {
        _unauthorizedError = true;
    }

    public ActionResult AsErrorResponse()
    {
        return ReturnErrorResponse();
    }
    
    // TODO: use  AsErrorResponse instead of ReturnErrorResponse.
    public ActionResult ReturnErrorResponse()
    {
        _errorResponseReturned = true;
        
        if (_unauthorizedError)
        {
            return new UnauthorizedResult();
        }

        if (_errors.Count > 0)
        {
            return new BadRequestObjectResult(_errors);
        }
        
        throw new InvalidOperationException("ValidationResult is empty, but ReturnErrorResponse was called. " +
                                                "This is likely a bug in the code, please check the logic.");
    }

    public List<ValidationError> GetErrors()
    {
        return _errors;
    }

    private readonly List<ValidationError> _errors = new();
    private bool _unauthorizedError;
    private bool _errorResponseReturned;

    public void Dispose()
    {
        if (_errors.Count > 0 && !_errorResponseReturned)
        {
#pragma warning disable CA1065
            throw new InvalidOperationException("Possible misuse of ValidationResult detected. \n " +
                                                "You should call ReturnBadRequest() from controller method " +
                                                "in case if you encountered any validation errors.");
#pragma warning restore CA1065
        }
    }
}