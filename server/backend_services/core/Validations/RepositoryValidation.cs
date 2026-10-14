using System.Text.RegularExpressions;
using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Services.GitHub;
using db.Entities;

namespace core.Validations;

public static partial class RepositoryValidation
{
    /// <summary>`owner/name`, or the repository's github.com URL.</summary>
    public static (string Owner, string Name)? ValidateFullName(string? fullName, ValidationResult vr)
    {
        if (string.IsNullOrWhiteSpace(fullName))
        {
            vr.ValueRequired("fullName", "This field is required.");
            return null;
        }

        var match = FullNamePattern().Match(fullName.Trim());
        if (!match.Success)
        {
            vr.IncorrectValue("fullName", "Use owner/name, such as acme/shop, or the repository's GitHub URL.");
            return null;
        }

        return (match.Groups["owner"].Value, match.Groups["name"].Value);
    }

    /// <summary>The repository as GitHub names it; one the token cannot see answers <c>doesnt-exist</c>.</summary>
    public static async Task<GitHubRepository?> TryGetGitHubRepository(string token, string owner, string name,
        ValidationResult vr, IGitHub github, CancellationToken cancellationToken)
    {
        var repository = await github.GetRepository(token, owner, name, cancellationToken);
        switch (repository.Failure)
        {
            case null:
                return repository.Value;
            case GitHubFailure.NotFound:
                vr.DoesntExist("fullName", "No such repository, or the connection's token cannot read it.");
                return null;
            case GitHubFailure.Unauthorized:
                vr.IncorrectState("connectionId", "GitHub no longer accepts this connection's token.");
                return null;
            default:
                vr.IncorrectState("fullName", repository.Message!);
                return null;
        }
    }

    public static async Task<bool> EnsureNotLinked(int projectId, GitHubRepository repository,
        ValidationResult vr, Read<RepositoryEntity> readRepositories)
    {
        if (!await RepositoryOps.IsLinked(projectId, repository.Owner, repository.Name, readRepositories))
        {
            return true;
        }

        vr.AlreadyExists("fullName", "The repository is already linked to this project.");
        return false;
    }

    /// <summary>
    /// The repository (tracked), its project, the caller's access to the organization, and whether the caller is
    /// in one of the project's teams. Another organization's repository answers <c>doesnt-exist</c>.
    /// </summary>
    public static async Task<(RepositoryEntity Repository, ProjectEntity Project, OrgAccess Access,
        bool IsProjectMember)?> TryGetRepositoryAccess(
        int repositoryId,
        int accountId,
        ValidationResult vr,
        Read<RepositoryEntity> readRepositories,
        Read<ProjectEntity> readProjects,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations,
        Read<ProjectTeamEntity> readProjectTeams,
        Read<TeamMemberEntity> readTeamMembers)
    {
        var repository = await RepositoryOps.TryGet(repositoryId, readRepositories);
        if (repository == null)
        {
            vr.DoesntExist("repositoryId", "Not found.");
            return null;
        }

        var project = (await ProjectOps.TryGet(repository.ProjectId, readProjects))!;
        var access = await OrgValidation.TryGetAccess(project.OrganizationId, accountId, vr, readMembers,
            readOrganizations, "repositoryId");
        if (access == null)
        {
            return null;
        }

        return (repository, project, access,
            await ProjectOps.IsMember(project.Id, accountId, readProjectTeams, readTeamMembers));
    }

    [GeneratedRegex(
        @"^(?:(?:https?://)?(?:www\.)?github\.com/)?(?<owner>[A-Za-z0-9](?:[A-Za-z0-9-]{0,38}))/(?<name>[A-Za-z0-9._-]{1,100}?)(?:\.git)?/?$")]
    private static partial Regex FullNamePattern();
}
