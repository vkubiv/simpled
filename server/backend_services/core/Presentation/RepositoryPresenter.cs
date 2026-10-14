using core.Libs.EntityAccess;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Presentation;

public class DeploymentPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string ApplicationName { get; set; }
    public required string? VersionConstraint { get; set; }
    public required string? Url { get; set; }

    /// <summary>The appspec's version on the default branch.</summary>
    public required string? RepoVersion { get; set; }

    /// <summary>unknown, up, down or mismatch.</summary>
    public required string Status { get; set; }

    public required DateTime? StatusSince { get; set; }
    public required string? LiveVersion { get; set; }
    public required DateTime? LiveDeployedAt { get; set; }
    public required DateTime? CheckedAt { get; set; }
    public required string? CheckError { get; set; }
}

public class EnvironmentPresentation
{
    public required int Id { get; set; }
    public required string Name { get; set; }
    public required string Path { get; set; }
    public required List<DeploymentPresentation> Deployments { get; set; }
}

public class RepositoryPresentation
{
    public required int Id { get; set; }
    public required int ProjectId { get; set; }
    public required int ConnectionId { get; set; }
    public required string Owner { get; set; }
    public required string Name { get; set; }
    public required string DefaultBranch { get; set; }
    public required DateTime CreatedAt { get; set; }
    public required DateTime? SyncedAt { get; set; }
    public required string? SyncError { get; set; }
    public required List<EnvironmentPresentation> Environments { get; set; }

    /// <summary>Sync and unlink: whoever may manage the project.</summary>
    public required bool CanManage { get; set; }
}

public class DeploymentEventPresentation
{
    public required int Id { get; set; }
    public required DateTime At { get; set; }

    /// <summary>version-changed, down, up or mismatch.</summary>
    public required string Kind { get; set; }

    public required string? Version { get; set; }
    public required string? PreviousVersion { get; set; }
    public required string? Details { get; set; }
}

public static class RepositoryPresenter
{
    public const int EventPageSize = 50;

    public static async Task<List<RepositoryPresentation>> PresentRepositories(
        int projectId,
        bool canManage,
        Read<RepositoryEntity> readRepositories,
        Read<DeployEnvironmentEntity> readEnvironments,
        Read<DeploymentEntity> readDeployments,
        CancellationToken cancellationToken)
    {
        var repositories = await readRepositories
            .Where(r => r.ProjectId == projectId)
            .OrderBy(r => r.Owner).ThenBy(r => r.Name)
            .ToListAsync(cancellationToken);
        return await Present(repositories, canManage, readEnvironments, readDeployments, cancellationToken);
    }

    public static async Task<RepositoryPresentation> PresentRepository(
        RepositoryEntity repository,
        bool canManage,
        Read<DeployEnvironmentEntity> readEnvironments,
        Read<DeploymentEntity> readDeployments,
        CancellationToken cancellationToken) =>
        (await Present([repository], canManage, readEnvironments, readDeployments, cancellationToken))[0];

    public static Task<List<DeploymentEventPresentation>> PresentEvents(
        int deploymentId,
        Read<DeploymentEventEntity> readEvents,
        CancellationToken cancellationToken) =>
        readEvents
            .Where(e => e.DeploymentId == deploymentId)
            .OrderByDescending(e => e.At).ThenByDescending(e => e.Id)
            .Take(EventPageSize)
            .Select(e => new DeploymentEventPresentation
            {
                Id = e.Id,
                At = e.At,
                Kind = e.Kind,
                Version = e.Version,
                PreviousVersion = e.PreviousVersion,
                Details = e.Details,
            })
            .ToListAsync(cancellationToken);

    private static async Task<List<RepositoryPresentation>> Present(
        List<RepositoryEntity> repositories,
        bool canManage,
        Read<DeployEnvironmentEntity> readEnvironments,
        Read<DeploymentEntity> readDeployments,
        CancellationToken cancellationToken)
    {
        var repositoryIds = repositories.Select(r => r.Id).ToList();
        var environments = await readEnvironments
            .Where(e => repositoryIds.Contains(e.RepositoryId))
            .OrderBy(e => e.Path)
            .ToListAsync(cancellationToken);
        var environmentIds = environments.Select(e => e.Id).ToList();
        var deployments = await readDeployments
            .Where(d => environmentIds.Contains(d.EnvironmentId))
            .OrderBy(d => d.Name)
            .ToListAsync(cancellationToken);

        return repositories.Select(r => new RepositoryPresentation
        {
            Id = r.Id,
            ProjectId = r.ProjectId,
            ConnectionId = r.GitHubConnectionId,
            Owner = r.Owner,
            Name = r.Name,
            DefaultBranch = r.DefaultBranch,
            CreatedAt = r.CreatedAt,
            SyncedAt = r.SyncedAt,
            SyncError = r.SyncError,
            Environments = environments.Where(e => e.RepositoryId == r.Id).Select(e => new EnvironmentPresentation
            {
                Id = e.Id,
                Name = e.Name,
                Path = e.Path,
                Deployments = deployments.Where(d => d.EnvironmentId == e.Id).Select(PresentDeployment).ToList(),
            }).ToList(),
            CanManage = canManage,
        }).ToList();
    }

    private static DeploymentPresentation PresentDeployment(DeploymentEntity d) => new()
    {
        Id = d.Id,
        Name = d.Name,
        ApplicationName = d.ApplicationName,
        VersionConstraint = d.VersionConstraint,
        Url = d.Url,
        RepoVersion = d.RepoVersion,
        Status = d.Status,
        StatusSince = d.StatusSince,
        LiveVersion = d.LiveVersion,
        LiveDeployedAt = d.LiveDeployedAt,
        CheckedAt = d.CheckedAt,
        CheckError = d.CheckError,
    };
}
