using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Primitive;
using db.Entities;

namespace core.Validations;

public static class AccountValidation
{
    public const int MinPasswordLength = 10;
    public const int MaxPasswordLength = 128;
    public const int MaxNameLength = 100;
    public const int MaxEmailLength = 254;

    public static bool ValidateEmail(string? email, ValidationResult vr)
    {
        if (string.IsNullOrWhiteSpace(email))
        {
            vr.ValueRequired("email", "Email is required.");
            return false;
        }

        var trimmed = email.Trim();
        if (trimmed.Length > MaxEmailLength ||
            !System.Net.Mail.MailAddress.TryCreate(trimmed, out var address) || address.Address != trimmed)
        {
            vr.IncorrectValue("email", "Email is not a valid address.");
            return false;
        }

        return true;
    }

    /// <summary>Length only, per NIST 800-63B: no composition rules, but not the email itself.</summary>
    public static bool ValidatePassword(string? password, string email, ValidationResult vr)
    {
        if (string.IsNullOrEmpty(password))
        {
            vr.ValueRequired("password", "Password is required.");
            return false;
        }

        if (password.Length < MinPasswordLength || password.Length > MaxPasswordLength)
        {
            vr.IncorrectValue("password",
                $"Password must be between {MinPasswordLength} and {MaxPasswordLength} characters.");
            return false;
        }

        if (string.Equals(password.Trim(), email.Trim(), StringComparison.OrdinalIgnoreCase))
        {
            vr.IncorrectValue("password", "Password must not be the same as the email.");
            return false;
        }

        return true;
    }

    public static bool ValidateName(string? name, ValidationResult vr) =>
        StringValidation.ValidateString(name?.Trim(), vr, "name", maxLength: MaxNameLength);

    public static bool ValidateRegistration(Registration registration, ValidationResult vr) =>
        ValidateEmail(registration.Email, vr) &&
        ValidatePassword(registration.Password, registration.Email, vr) &&
        ValidateName(registration.Name, vr) &&
        OrganizationValidation.ValidateName(registration.OrganizationName, vr, "organizationName");

    public static async Task<bool> EnsureAccountNotExist(string email, ValidationResult vr,
        Read<AccountEntity> readAccounts)
    {
        if (await AccountOps.TryGetAccountByLogin(email, readAccounts: readAccounts) == null)
        {
            return true;
        }

        vr.AlreadyExists("email", "An account with this email already exists.");
        return false;
    }
}
