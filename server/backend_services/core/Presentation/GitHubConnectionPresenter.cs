using core.Libs.EntityAccess;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

/// <summary>A connection without its token, which never leaves the server.</summary>
public class GitHubConnectionPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string GitHubLogin { get; set; }
    public required DateTime CreatedAt { get; set; }
    public required int RepositoryCount { get; set; }
}

public static class GitHubConnectionPresenter
{
    public static async Task<List<GitHubConnectionPresentation>> PresentConnections(
        int organizationId,
        Read<GitHubConnectionEntity> readConnections,
        Read<RepositoryEntity> readRepositories,
        CancellationToken cancellationToken)
    {
        var connections = await readConnections
            .Where(c => c.OrganizationId == organizationId)
            .OrderBy(c => c.Name)
            .ToListAsync(cancellationToken);
        var ids = connections.Select(c => c.Id).ToList();
        var counts = await readRepositories
            .Where(r => ids.Contains(r.GitHubConnectionId))
            .GroupBy(r => r.GitHubConnectionId)
            .Select(g => new { Id = g.Key, Count = g.Count() })
            .ToDictionaryAsync(x => x.Id, x => x.Count, cancellationToken);
        return connections.Select(c => Present(c, counts.GetValueOrDefault(c.Id))).ToList();
    }

    public static async Task<GitHubConnectionPresentation> PresentConnection(
        GitHubConnectionEntity connection,
        Read<RepositoryEntity> readRepositories,
        CancellationToken cancellationToken) =>
        Present(connection,
            await readRepositories.CountAsync(r => r.GitHubConnectionId == connection.Id, cancellationToken));

    private static GitHubConnectionPresentation Present(GitHubConnectionEntity connection, int repositoryCount) =>
        new()
        {
            Id = connection.Id,
            Name = connection.Name,
            GitHubLogin = connection.GitHubLogin,
            CreatedAt = connection.CreatedAt,
            RepositoryCount = repositoryCount,
        };
}
