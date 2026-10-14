using core.Libs.MVCValidation;

namespace core.Validations;

public static class StringValidation
{
    public static bool ValidateString(string? str, ValidationResult vr, string fieldName, int minLength = 1,
        int maxLength = 100)
    {
        if (string.IsNullOrEmpty(str))
        {
            vr.ValueRequired( fieldName, "This field is required.");
            return false;
        }
        
        if (str.Length < minLength)
        {
            vr.IncorrectValue(fieldName, "min length is " + minLength);
            return false;
        }

        if (str.Length > maxLength)
        {
            vr.IncorrectValue(fieldName, "max length is " + maxLength);
            return false;
        }

        return true;
    }
}