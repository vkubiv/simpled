using core.Libs.MVCValidation;

namespace core.Validations;

public static class OrganizationValidation
{
    public const int MaxNameLength = 100;

    public static bool ValidateName(string? name, ValidationResult vr, string fieldName = "name") =>
        StringValidation.ValidateString(name?.Trim(), vr, fieldName, maxLength: MaxNameLength);
}
