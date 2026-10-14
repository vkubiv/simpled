using core.Services.Monitor;

namespace utest.TestSupport.TestDoubles;

/// <summary>Answers per base URL what a test sets; an unknown URL does not answer.</summary>
public class FakeVersionProbe : IVersionProbe
{
    private readonly Dictionary<string, ProbeResult> _answers = [];

    public List<string> Probed { get; } = [];

    public void Answer(string baseUrl, string application, string deployment, string version) =>
        _answers[baseUrl] = ProbeResult.Ok(new VersionInfo(application, version, deployment,
            new DateTime(2026, 10, 1, 0, 0, 0, DateTimeKind.Utc)));

    public void Fail(string baseUrl) => _answers.Remove(baseUrl);

    public Task<ProbeResult> Probe(string baseUrl, CancellationToken cancellationToken)
    {
        Probed.Add(baseUrl);
        return Task.FromResult(_answers.GetValueOrDefault(baseUrl) ??
                               ProbeResult.Fail("The version endpoint answered 502."));
    }
}
