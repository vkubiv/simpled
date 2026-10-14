using core.Primitive;

namespace core.Rules;

/// <summary>
/// Who may do what in an organization (docs/plan.md, "Roles"). Pure: callers look up the role and team
/// membership, validations turn a false into a <c>forbidden</c> error, presenters expose the answers to the UI.
/// "At least one owner" needs data and lives in OrgValidation.EnsureNotLastOwner.
/// </summary>
public static class OrgPermissions
{
    public static bool CanManageOrganization(OrgRole role) => role == OrgRole.Owner;

    /// <summary>Invite, remove members and change roles at all; the role-specific checks are below.</summary>
    public static bool CanManageMembers(OrgRole role) => role is OrgRole.Owner or OrgRole.Admin;

    /// <summary>Roles <paramref name="role"/> may invite with or assign: owners any, admins anything but owner.</summary>
    public static IReadOnlyList<OrgRole> AssignableRoles(OrgRole role) => role switch
    {
        OrgRole.Owner => [OrgRole.Owner, OrgRole.Admin, OrgRole.Manager, OrgRole.Member],
        OrgRole.Admin => [OrgRole.Admin, OrgRole.Manager, OrgRole.Member],
        _ => [],
    };

    public static bool CanAssign(OrgRole role, OrgRole newRole) => AssignableRoles(role).Contains(newRole);

    /// <summary>Changing someone's role, or removing them: only an owner touches an owner.</summary>
    public static bool CanManageMember(OrgRole role, OrgRole targetRole) =>
        CanManageMembers(role) && (targetRole != OrgRole.Owner || role == OrgRole.Owner);

    public static bool CanChangeRole(OrgRole role, OrgRole targetRole, OrgRole newRole) =>
        CanManageMember(role, targetRole) && CanAssign(role, newRole);

    /// <summary>Anyone may leave; removing someone else needs <see cref="CanManageMember"/>.</summary>
    public static bool CanRemoveMember(OrgRole role, OrgRole targetRole, bool isSelf) =>
        isSelf || CanManageMember(role, targetRole);

    public static bool CanCreateTeam(OrgRole role) => role is OrgRole.Owner or OrgRole.Admin or OrgRole.Manager;

    /// <summary>A manager's new project must link one of their teams, which <see cref="CanManageTeam"/> checks.</summary>
    public static bool CanCreateProject(OrgRole role) => CanCreateTeam(role);

    /// <summary>Rename, delete, change members and linked projects of a team.</summary>
    public static bool CanManageTeam(OrgRole role, bool isTeamMember) =>
        role is OrgRole.Owner or OrgRole.Admin || (role == OrgRole.Manager && isTeamMember);

    /// <summary>Rename, delete and change linked teams of a project; a manager needs one of its teams.</summary>
    public static bool CanManageProject(OrgRole role, bool isProjectMember) =>
        role is OrgRole.Owner or OrgRole.Admin || (role == OrgRole.Manager && isProjectMember);
}
