using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Primitive;
using db.Entities;

namespace core.Validations;

public static class ProjectValidation
{
    public const int MaxNameLength = 100;

    public static bool ValidateName(string? name, ValidationResult vr) =>
        StringValidation.ValidateString(name?.Trim(), vr, "name", maxLength: MaxNameLength);

    /// <summary>Empty is fine where the slug is optional; otherwise lower-case letters, digits and dashes.</summary>
    public static bool ValidateSlug(string? slug, ValidationResult vr, bool required)
    {
        if (string.IsNullOrWhiteSpace(slug))
        {
            if (required)
            {
                vr.ValueRequired("slug", "Slug is required.");
            }

            return !required;
        }

        if (!Slug.IsValid(slug.Trim()))
        {
            vr.IncorrectValue("slug",
                $"Use lower-case letters, digits and single dashes, at most {Slug.MaxLength} characters.");
            return false;
        }

        return true;
    }

    public static async Task<bool> EnsureSlugFree(int organizationId, string? slug, int? exceptProjectId,
        ValidationResult vr, Read<ProjectEntity> readProjects)
    {
        if (string.IsNullOrWhiteSpace(slug) ||
            !await ProjectOps.IsSlugTaken(organizationId, slug.Trim(), exceptProjectId, readProjects))
        {
            return true;
        }

        vr.AlreadyExists("slug", "A project with this slug already exists.");
        return false;
    }

    /// <summary>
    /// The project (tracked), the caller's access to its organization, and whether the caller is in one of its
    /// teams. A project of an organization the caller is not in answers <c>doesnt-exist</c> on "projectId".
    /// </summary>
    public static async Task<(ProjectEntity Project, OrgAccess Access, bool IsProjectMember)?> TryGetProjectAccess(
        int projectId,
        int accountId,
        ValidationResult vr,
        Read<ProjectEntity> readProjects,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamMemberEntity> readTeamMembers)
    {
        var project = await TryGetProject(projectId, vr, readProjects);
        var access = project == null
            ? null
            : await OrgValidation.TryGetAccess(project.OrganizationId, accountId, vr, readMembers,
                readOrganizations, "projectId");
        if (access == null)
        {
            return null;
        }

        return (project!, access,
            await ProjectOps.IsMember(project!.Id, accountId, readProjectTeams, readTeamMembers));
    }

    public static async Task<ProjectEntity?> TryGetProject(int projectId, ValidationResult vr,
        Read<ProjectEntity> readProjects)
    {
        var project = await ProjectOps.TryGet(projectId, readProjects);
        if (project == null)
        {
            vr.DoesntExist("projectId", "Not found.");
        }

        return project;
    }
}
