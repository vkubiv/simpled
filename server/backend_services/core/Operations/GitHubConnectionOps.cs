using core.Libs.EntityAccess;
using core.Services.GitHub;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class GitHubConnectionOps
{
    public static Task<GitHubConnectionEntity?> TryGet(int connectionId,
        Read<GitHubConnectionEntity> readConnections) =>
        readConnections.FirstOrDefaultAsync(c => c.Id == connectionId);

    public static async Task<GitHubConnectionEntity> Create(
        int organizationId,
        string name,
        string token,
        GitHubUser user,
        DateTime now,
        TokenCipher cipher,
        Mutate<GitHubConnectionEntity> mutateConnections)
    {
        var connection = new GitHubConnectionEntity
        {
            OrganizationId = organizationId,
            Name = name.Trim(),
            GitHubLogin = user.Login,
            TokenCiphertext = cipher.Encrypt(token.Trim()),
            CreatedAt = now,
        };
        await mutateConnections.AddAsync(connection);
        return connection;
    }

    public static void ReplaceToken(GitHubConnectionEntity connection, string token, GitHubUser user,
        TokenCipher cipher)
    {
        connection.TokenCiphertext = cipher.Encrypt(token.Trim());
        connection.GitHubLogin = user.Login;
    }

    public static string Token(GitHubConnectionEntity connection, TokenCipher cipher) =>
        cipher.Decrypt(connection.TokenCiphertext);

    public static Task<bool> IsNameTaken(int organizationId, string name, int? exceptConnectionId,
        Read<GitHubConnectionEntity> readConnections) =>
        readConnections.AnyAsync(c =>
            c.OrganizationId == organizationId && c.Name == name && c.Id != exceptConnectionId);

    public static Task<int> RepositoryCount(int connectionId, Read<RepositoryEntity> readRepositories) =>
        readRepositories.CountAsync(r => r.GitHubConnectionId == connectionId);
}
