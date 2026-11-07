namespace core.Services.Monitor;

/// <summary>What a deployment's version endpoint answers.</summary>
public record VersionInfo(string Application, string Version, string Deployment, DateTime? DeployedAt);

/// <summary>What the simpled controller answers at `/simpled/health/{deployment}` (Docker, simpled 2.1+).</summary>
/// <param name="Status">`healthy`, `degraded` or `down`.</param>
/// <param name="Converged">False while a rollout is in progress.</param>
/// <param name="Tls">The worst certificate of the deployment's domains, on Swarm with Let's Encrypt.</param>
public record HealthInfo(string Status, bool Converged, string? Tls);

/// <summary>The endpoints' answers, or why there are none.</summary>
/// <param name="Health">Null when the deployment has no health endpoint.</param>
public record ProbeResult(VersionInfo? Info, HealthInfo? Health, string? Error)
{
    public static ProbeResult Ok(VersionInfo info, HealthInfo? health = null) => new(info, health, null);

    public static ProbeResult Fail(string error) => new(null, null, error);
}

public interface IDeploymentProbe
{
    /// <param name="versionUrl">The version endpoint, such as https://ops.example.com/simpled/version/prod.</param>
    /// <param name="healthUrl">The controller's health endpoint for the deployment, if it has one.</param>
    Task<ProbeResult> Probe(string versionUrl, string? healthUrl, CancellationToken cancellationToken);
}
