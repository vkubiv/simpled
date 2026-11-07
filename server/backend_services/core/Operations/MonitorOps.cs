using core.Libs.EntityAccess;
using core.Services.Monitor;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

/// <summary>What a check of a deployment's endpoints means for its status, and the events it leaves.</summary>
public static class MonitorOps
{
    /// <summary>
    /// Failed or unhealthy checks in a row before a deployment counts as down or degraded; one is a network blip
    /// or a rollout replacing a task.
    /// </summary>
    public const int FailuresBeforeDown = 2;

    /// <summary>Deployments with an address whose last check is older than <paramref name="interval"/>.</summary>
    public static Task<List<DeploymentEntity>> DueForCheck(DateTime now, TimeSpan interval, int limit,
        Read<DeploymentEntity> readDeployments)
    {
        var before = now - interval;
        return readDeployments
            .Where(d => d.VersionUrl != null && (d.CheckedAt == null || d.CheckedAt <= before))
            .OrderBy(d => d.CheckedAt)
            .Take(limit)
            .ToListAsync();
    }

    /// <summary>Forgets what was seen, for a deployment whose endpoints moved.</summary>
    public static void Reset(DeploymentEntity deployment)
    {
        deployment.Status = DeploymentEntity.StatusUnknown;
        deployment.StatusSince = null;
        deployment.LiveVersion = null;
        deployment.LiveDeployedAt = null;
        deployment.CheckedAt = null;
        deployment.CheckError = null;
        deployment.FailureCount = 0;
    }

    public static async Task ApplyCheck(
        DeploymentEntity deployment,
        ProbeResult result,
        DateTime now,
        Mutate<DeploymentEventEntity> mutateEvents)
    {
        deployment.CheckedAt = now;

        if (result.Info is not { } info)
        {
            deployment.FailureCount++;
            deployment.CheckError = result.Error;
            if (deployment.FailureCount >= FailuresBeforeDown)
            {
                await SetStatus(deployment, DeploymentEntity.StatusDown, now, result.Error, mutateEvents);
            }

            return;
        }

        if (info.Application != deployment.ApplicationName || info.Deployment != deployment.Name)
        {
            deployment.FailureCount = 0;
            deployment.CheckError = $"The endpoint answers for {info.Application}/{info.Deployment}, " +
                                    $"not {deployment.ApplicationName}/{deployment.Name}.";
            await SetStatus(deployment, DeploymentEntity.StatusMismatch, now, deployment.CheckError, mutateEvents);
            return;
        }

        deployment.CheckError = null;
        if (deployment.LiveVersion != null && deployment.LiveVersion != info.Version)
        {
            await mutateEvents.AddAsync(new DeploymentEventEntity
            {
                Deployment = deployment,
                At = now,
                Kind = DeploymentEventEntity.KindVersionChanged,
                Version = info.Version,
                PreviousVersion = deployment.LiveVersion,
            });
        }

        deployment.LiveVersion = info.Version;
        deployment.LiveDeployedAt = info.DeployedAt;

        if (result.Health is { Status: not "healthy" } health)
        {
            deployment.FailureCount++;
            deployment.CheckError = HealthProblem(health);
            if (deployment.FailureCount >= FailuresBeforeDown)
            {
                await SetStatus(deployment,
                    health.Status == "down" ? DeploymentEntity.StatusDown : DeploymentEntity.StatusDegraded,
                    now, deployment.CheckError, mutateEvents);
            }

            return;
        }

        deployment.FailureCount = 0;
        await SetStatus(deployment, DeploymentEntity.StatusUp, now, null, mutateEvents);
    }

    private static string HealthProblem(HealthInfo health)
    {
        var notes = new List<string>();
        if (health.Tls is { } tls && tls != "valid")
        {
            notes.Add($"certificate {tls}");
        }

        if (!health.Converged)
        {
            notes.Add("rollout in progress");
        }

        return $"The controller reports the deployment {health.Status}" +
               (notes.Count == 0 ? "." : $" ({string.Join(", ", notes)}).");
    }

    /// <summary>Records the change; the first answer of a new deployment is not news, a first failure is.</summary>
    private static async Task SetStatus(DeploymentEntity deployment, string status, DateTime now, string? details,
        Mutate<DeploymentEventEntity> mutateEvents)
    {
        if (deployment.Status == status)
        {
            return;
        }

        var previous = deployment.Status;
        deployment.Status = status;
        deployment.StatusSince = now;
        if (previous == DeploymentEntity.StatusUnknown && status == DeploymentEntity.StatusUp)
        {
            return;
        }

        await mutateEvents.AddAsync(new DeploymentEventEntity
        {
            Deployment = deployment,
            At = now,
            Kind = status switch
            {
                DeploymentEntity.StatusUp => DeploymentEventEntity.KindUp,
                DeploymentEntity.StatusDown => DeploymentEventEntity.KindDown,
                DeploymentEntity.StatusDegraded => DeploymentEventEntity.KindDegraded,
                _ => DeploymentEventEntity.KindMismatch,
            },
            Version = deployment.LiveVersion,
            Details = details,
        });
    }
}
