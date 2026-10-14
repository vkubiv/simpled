using core.Libs.EntityAccess;
using core.Libs.MVCValidation;
using core.Operations;
using core.Services.GitHub;
using db.Entities;

namespace core.Validations;

public static class GitHubConnectionValidation
{
    public const int MaxNameLength = 100;
    public const int MaxTokenLength = 400;

    public static bool ValidateName(string? name, ValidationResult vr) =>
        StringValidation.ValidateString(name?.Trim(), vr, "name", maxLength: MaxNameLength);

    public static bool ValidateToken(string? token, ValidationResult vr) =>
        StringValidation.ValidateString(token?.Trim(), vr, "token", maxLength: MaxTokenLength);

    public static async Task<bool> EnsureNameFree(int organizationId, string name, int? exceptConnectionId,
        ValidationResult vr, Read<GitHubConnectionEntity> readConnections)
    {
        if (!await GitHubConnectionOps.IsNameTaken(organizationId, name.Trim(), exceptConnectionId, readConnections))
        {
            return true;
        }

        vr.AlreadyExists("name", "A GitHub connection with this name already exists.");
        return false;
    }

    /// <summary>The account the token belongs to; a token GitHub refuses is an incorrect "token".</summary>
    public static async Task<GitHubUser?> TryGetTokenUser(string token, ValidationResult vr, IGitHub github,
        CancellationToken cancellationToken)
    {
        var user = await github.GetUser(token.Trim(), cancellationToken);
        switch (user.Failure)
        {
            case null:
                return user.Value;
            case GitHubFailure.Unavailable:
                vr.IncorrectState("token", user.Message!);
                return null;
            default:
                vr.IncorrectValue("token", "GitHub does not accept this token.");
                return null;
        }
    }

    /// <summary>A connection of <paramref name="organizationId"/>; any other answers <c>doesnt-exist</c>.</summary>
    public static async Task<GitHubConnectionEntity?> TryGetConnection(int organizationId, int connectionId,
        ValidationResult vr, Read<GitHubConnectionEntity> readConnections, string fieldName = "connectionId")
    {
        var connection = await GitHubConnectionOps.TryGet(connectionId, readConnections);
        if (connection == null || connection.OrganizationId != organizationId)
        {
            vr.DoesntExist(fieldName, "Not found.");
            return null;
        }

        return connection;
    }

    /// <summary>The connection (tracked) and the caller's access to its organization.</summary>
    public static async Task<(GitHubConnectionEntity Connection, OrgAccess Access)?> TryGetConnectionAccess(
        int connectionId,
        int accountId,
        ValidationResult vr,
        Read<GitHubConnectionEntity> readConnections,
        Read<OrgMemberEntity> readMembers,
        Read<OrganizationEntity> readOrganizations)
    {
        var connection = await GitHubConnectionOps.TryGet(connectionId, readConnections);
        if (connection == null)
        {
            vr.DoesntExist("connectionId", "Not found.");
            return null;
        }

        var access = await OrgValidation.TryGetAccess(connection.OrganizationId, accountId, vr, readMembers,
            readOrganizations, "connectionId");
        return access == null ? null : (connection, access);
    }

    public static async Task<bool> EnsureUnused(GitHubConnectionEntity connection, ValidationResult vr,
        Read<RepositoryEntity> readRepositories)
    {
        var count = await GitHubConnectionOps.RepositoryCount(connection.Id, readRepositories);
        if (count == 0)
        {
            return true;
        }

        vr.IncorrectState("connectionId",
            $"{count} linked {(count == 1 ? "repository uses" : "repositories use")} this connection.");
        return false;
    }
}
