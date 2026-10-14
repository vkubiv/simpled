namespace core.Services.Monitor;

/// <summary>What a simpled gateway answers at `/.well-known/simpled/version`.</summary>
public record VersionInfo(string Application, string Version, string Deployment, DateTime? DeployedAt);

/// <summary>The endpoint's answer, or why there is none.</summary>
public record ProbeResult(VersionInfo? Info, string? Error)
{
    public static ProbeResult Ok(VersionInfo info) => new(info, null);

    public static ProbeResult Fail(string error) => new(null, error);
}

public interface IVersionProbe
{
    /// <param name="baseUrl">The deployment's primary host, such as https://shop.example.com.</param>
    Task<ProbeResult> Probe(string baseUrl, CancellationToken cancellationToken);
}
