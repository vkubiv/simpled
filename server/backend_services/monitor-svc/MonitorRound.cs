using core.Libs.EntityAccess;
using core.Operations;
using core.Services.GitHub;
using core.Services.Monitor;
using db.Entities;

namespace monitor_svc;

/// <summary>The work of one monitor round, apart from scheduling and scopes so tests can drive it.</summary>
public static class MonitorRound
{
    public const int SyncBatchSize = 20;
    public const int CheckBatchSize = 200;
    public const int ParallelProbes = 16;

    /// <summary>Re-reads one repository's spec files and saves; a repository unlinked meanwhile is skipped.</summary>
    public static async Task SyncRepository(
        int repositoryId,
        DateTime now,
        IGitHub github,
        TokenCipher cipher,
        Read<RepositoryEntity> readRepositories,
        Read<GitHubConnectionEntity> readConnections,
        Mutate<DeployEnvironmentEntity> mutateEnvironments,
        Mutate<DeploymentEntity> mutateDeployments,
        UnitOfWork unitOfWork,
        CancellationToken cancellationToken)
    {
        var repository = await RepositoryOps.TryGet(repositoryId, readRepositories);
        var connection = repository == null
            ? null
            : await GitHubConnectionOps.TryGet(repository.GitHubConnectionId, readConnections);
        if (repository == null || connection == null)
        {
            return;
        }

        await RepositoryOps.Sync(
            repository: repository,
            token: GitHubConnectionOps.Token(connection, cipher),
            github: github,
            now: now,
            mutateEnvironments: mutateEnvironments,
            mutateDeployments: mutateDeployments,
            cancellationToken: cancellationToken);
        await unitOfWork.SaveChangesAsync(cancellationToken);
    }

    /// <summary>Checks every deployment that is due, in parallel, then records the answers in one save.</summary>
    /// <returns>How many deployments were checked.</returns>
    public static async Task<int> CheckDeployments(
        DateTime now,
        TimeSpan interval,
        IDeploymentProbe probe,
        Mutate<DeploymentEntity> mutateDeployments,
        Mutate<DeploymentEventEntity> mutateEvents,
        UnitOfWork unitOfWork,
        CancellationToken cancellationToken)
    {
        var deployments = await MonitorOps.DueForCheck(now, interval, CheckBatchSize, mutateDeployments.Read);
        if (deployments.Count == 0)
        {
            return 0;
        }

        using var gate = new SemaphoreSlim(ParallelProbes);
        var results = await Task.WhenAll(deployments.Select(async d =>
        {
            await gate.WaitAsync(cancellationToken);
            try
            {
                return await probe.Probe(d.VersionUrl!, d.HealthUrl, cancellationToken);
            }
            finally
            {
                gate.Release();
            }
        }));

        for (var i = 0; i < deployments.Count; i++)
        {
            await MonitorOps.ApplyCheck(deployments[i], results[i], now, mutateEvents);
        }

        await unitOfWork.SaveChangesAsync(cancellationToken);
        return deployments.Count;
    }
}
