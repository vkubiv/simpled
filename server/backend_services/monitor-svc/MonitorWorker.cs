using config;
using core.Libs.EntityAccess;
using core.Operations;
using core.Services;
using core.Services.GitHub;
using core.Services.Monitor;
using db.Entities;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;

namespace monitor_svc;

/// <summary>
/// Runs a monitor round every tick: re-syncs repositories whose specs are stale, then checks deployments whose
/// last check is older than the interval. Each repository syncs in its own scope, so one that fails to save does
/// not take the others with it. Run one instance: two would check everything twice.
/// </summary>
public sealed partial class MonitorWorker(
    IServiceScopeFactory scopes,
    CommonConfiguration config,
    MonitorHeartbeat heartbeat,
    ILogger<MonitorWorker> logger) : BackgroundService
{
    /// <summary>Half the check interval, between 1 and 15 seconds: a due deployment waits at most that long.</summary>
    public static TimeSpan Tick(CommonConfiguration config) =>
        TimeSpan.FromSeconds(Math.Clamp(config.MonitorInterval.TotalSeconds / 2, 1, 15));

    protected override async Task ExecuteAsync(CancellationToken stoppingToken)
    {
        var tick = Tick(config);
        LogStarted(config.MonitorInterval, config.RepositorySyncInterval);

        while (!stoppingToken.IsCancellationRequested)
        {
            try
            {
                await SyncRepositories(stoppingToken);
                await CheckDeployments(stoppingToken);
            }
            catch (Exception e) when (e is not OperationCanceledException)
            {
                LogRoundFailed(e);
            }

            // A failed round still beats: the database may simply not be migrated yet, and db-setup runs after
            // the services are healthy. Failures go to the log (and Sentry); /health only says the loop runs.
            heartbeat.Beat(DateTime.UtcNow);

            await Task.Delay(tick, stoppingToken);
        }
    }

    private async Task SyncRepositories(CancellationToken cancellationToken)
    {
        List<int> due;
        using (var scope = scopes.CreateScope())
        {
            var now = scope.ServiceProvider.GetRequiredService<Clock>().Now;
            due = (await RepositoryOps.DueForSync(now, config.RepositorySyncInterval, MonitorRound.SyncBatchSize,
                    scope.ServiceProvider.GetRequiredService<Read<RepositoryEntity>>()))
                .Select(r => r.Id)
                .ToList();
        }

        foreach (var repositoryId in due)
        {
            using var scope = scopes.CreateScope();
            var services = scope.ServiceProvider;
            try
            {
                await MonitorRound.SyncRepository(
                    repositoryId: repositoryId,
                    now: services.GetRequiredService<Clock>().Now,
                    github: services.GetRequiredService<IGitHub>(),
                    cipher: services.GetRequiredService<TokenCipher>(),
                    readRepositories: services.GetRequiredService<Read<RepositoryEntity>>(),
                    readConnections: services.GetRequiredService<Read<GitHubConnectionEntity>>(),
                    mutateEnvironments: services.GetRequiredService<Mutate<DeployEnvironmentEntity>>(),
                    mutateDeployments: services.GetRequiredService<Mutate<DeploymentEntity>>(),
                    unitOfWork: services.GetRequiredService<UnitOfWork>(),
                    cancellationToken: cancellationToken);
            }
            catch (Exception e) when (e is not OperationCanceledException)
            {
                LogSyncFailed(repositoryId, e);
            }
        }
    }

    private async Task CheckDeployments(CancellationToken cancellationToken)
    {
        using var scope = scopes.CreateScope();
        var services = scope.ServiceProvider;
        await MonitorRound.CheckDeployments(
            now: services.GetRequiredService<Clock>().Now,
            interval: config.MonitorInterval,
            probe: services.GetRequiredService<IVersionProbe>(),
            mutateDeployments: services.GetRequiredService<Mutate<DeploymentEntity>>(),
            mutateEvents: services.GetRequiredService<Mutate<DeploymentEventEntity>>(),
            unitOfWork: services.GetRequiredService<UnitOfWork>(),
            cancellationToken: cancellationToken);
    }

    [LoggerMessage(Level = LogLevel.Information,
        Message = "Monitoring: deployments every {MonitorInterval}, repositories every {SyncInterval}")]
    private partial void LogStarted(TimeSpan monitorInterval, TimeSpan syncInterval);

    [LoggerMessage(Level = LogLevel.Error, Message = "Monitor round failed")]
    private partial void LogRoundFailed(Exception exception);

    [LoggerMessage(Level = LogLevel.Error, Message = "Sync of repository {RepositoryId} failed")]
    private partial void LogSyncFailed(int repositoryId, Exception exception);
}
