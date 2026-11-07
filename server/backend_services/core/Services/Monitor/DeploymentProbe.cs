using System.Net;
using System.Text.Json;

namespace core.Services.Monitor;

public sealed class DeploymentProbe : IDeploymentProbe
{
    private readonly HttpClient _http;

    public DeploymentProbe(HttpClient http)
    {
        _http = http;
    }

    /// <summary>An HttpClient for probes: short timeout, no redirects, through <paramref name="proxyUrl"/> if set.</summary>
    public static HttpClient CreateHttpClient(string? proxyUrl)
    {
#pragma warning disable CA2000 // the HttpClient owns the handler
        var handler = new HttpClientHandler { AllowAutoRedirect = false, CheckCertificateRevocationList = true };
#pragma warning restore CA2000
        if (proxyUrl != null)
        {
            handler.Proxy = new WebProxy(proxyUrl);
            handler.UseProxy = true;
        }

        return new HttpClient(handler) { Timeout = TimeSpan.FromSeconds(10) };
    }

    public async Task<ProbeResult> Probe(string versionUrl, string? healthUrl, CancellationToken cancellationToken)
    {
        var version = await Read(versionUrl, "version", [HttpStatusCode.OK], cancellationToken);
        if (version.Error != null)
        {
            return ProbeResult.Fail(version.Error);
        }

        // The controller answers health with the same document as 200 when healthy and 503 otherwise.
        var health = healthUrl == null
            ? null
            : await Read(healthUrl, "health", [HttpStatusCode.OK, HttpStatusCode.ServiceUnavailable],
                cancellationToken);
        if (health?.Error != null)
        {
            version.Json!.Dispose();
            return ProbeResult.Fail(health.Error);
        }

        try
        {
            using var versionJson = version.Json!;
            using var healthJson = health?.Json;
            var root = versionJson.RootElement;
            DateTime? deployedAt = root.TryGetProperty("deployed_at", out var at) && at.ValueKind == JsonValueKind.Number
                ? DateTimeOffset.FromUnixTimeSeconds(at.GetInt64()).UtcDateTime
                : null;
            var info = new VersionInfo(
                root.GetProperty("application").GetString() ?? "",
                root.GetProperty("version").GetString() ?? "",
                root.GetProperty("deployment").GetString() ?? "",
                deployedAt);
            return ProbeResult.Ok(info, healthJson == null ? null : ReadHealth(healthJson.RootElement));
        }
        catch (Exception e) when (e is KeyNotFoundException or InvalidOperationException or FormatException)
        {
            return ProbeResult.Fail($"The {(healthUrl == null ? "version endpoint" : "controller")} answered " +
                                    $"something else: {e.Message}");
        }
    }

    private static HealthInfo ReadHealth(JsonElement root) => new(
        root.GetProperty("status").GetString() ?? "",
        root.TryGetProperty("converged", out var converged) && converged.ValueKind == JsonValueKind.True,
        root.TryGetProperty("tls", out var tls) && tls.ValueKind == JsonValueKind.String ? tls.GetString() : null);

    private sealed record Answer(JsonDocument? Json, string? Error);

    private async Task<Answer> Read(string url, string what, HttpStatusCode[] accepted,
        CancellationToken cancellationToken)
    {
        try
        {
            using var response = await _http.GetAsync(new Uri(url), cancellationToken);
            if (!accepted.Contains(response.StatusCode))
            {
                return new Answer(null, response.StatusCode == HttpStatusCode.NotFound && what == "health"
                    ? "The controller does not know the deployment (404)."
                    : $"The {what} endpoint answered {(int)response.StatusCode}.");
            }

            await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken);
            return new Answer(await JsonDocument.ParseAsync(stream, cancellationToken: cancellationToken), null);
        }
        catch (Exception e) when (e is HttpRequestException or JsonException or UriFormatException ||
                                  (e is TaskCanceledException && !cancellationToken.IsCancellationRequested))
        {
            return new Answer(null, e is TaskCanceledException
                ? $"The {what} endpoint did not answer in time."
                : $"The {what} endpoint could not be read: {e.Message}");
        }
    }
}
