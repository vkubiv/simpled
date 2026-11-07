using core.Services.Monitor;

namespace utest.TestSupport.TestDoubles;

/// <summary>Answers per version URL what a test sets; an unknown URL does not answer.</summary>
public class FakeDeploymentProbe : IDeploymentProbe
{
    private readonly Dictionary<string, ProbeResult> _answers = [];

    /// <summary>The version URLs asked, with the health URL that came along.</summary>
    public List<(string VersionUrl, string? HealthUrl)> Probed { get; } = [];

    public void Answer(string versionUrl, string application, string deployment, string version,
        string? health = null) =>
        _answers[versionUrl] = ProbeResult.Ok(
            new VersionInfo(application, version, deployment, new DateTime(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc)),
            health == null ? null : new HealthInfo(health, Converged: true, Tls: null));

    public void Fail(string versionUrl) => _answers.Remove(versionUrl);

    public Task<ProbeResult> Probe(string versionUrl, string? healthUrl, CancellationToken cancellationToken)
    {
        Probed.Add((versionUrl, healthUrl));
        return Task.FromResult(_answers.GetValueOrDefault(versionUrl) ??
                               ProbeResult.Fail("The version endpoint answered 502."));
    }
}
