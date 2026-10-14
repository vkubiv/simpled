namespace core.Primitive;

/// <summary>A member's role in one organization, strongest first. Stored kebab-case in org_members.role.</summary>
public enum OrgRole
{
    /// <summary>Everything, including the organization's settings, deleting it, and granting Owner.</summary>
    Owner,

    /// <summary>Admin on every team; adds and removes members.</summary>
    Admin,

    /// <summary>Creates teams and projects; manages the teams they belong to and those teams' projects.</summary>
    Manager,

    /// <summary>Views the organization's data.</summary>
    Member,
}

public static class OrgRoles
{
    public static string ToValue(OrgRole role) => role switch
    {
        OrgRole.Owner => "owner",
        OrgRole.Admin => "admin",
        OrgRole.Manager => "manager",
        OrgRole.Member => "member",
        _ => throw new ArgumentOutOfRangeException(nameof(role), role, null),
    };

    public static OrgRole? TryParse(string? value) => value switch
    {
        "owner" => OrgRole.Owner,
        "admin" => OrgRole.Admin,
        "manager" => OrgRole.Manager,
        "member" => OrgRole.Member,
        _ => null,
    };

    public static OrgRole Parse(string value) =>
        TryParse(value) ?? throw new InvalidOperationException($"Unknown organization role '{value}'");
}
