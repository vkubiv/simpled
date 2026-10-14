using System.Net;
using System.Text.Json;

namespace core.Services.Monitor;

public sealed class VersionProbe : IVersionProbe
{
    public const string Path = "/.well-known/simpled/version";

    private readonly HttpClient _http;

    public VersionProbe(HttpClient http)
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

    public async Task<ProbeResult> Probe(string baseUrl, CancellationToken cancellationToken)
    {
        try
        {
            using var response = await _http.GetAsync(new Uri(baseUrl.TrimEnd('/') + Path), cancellationToken);
            if (response.StatusCode != HttpStatusCode.OK)
            {
                return ProbeResult.Fail($"The version endpoint answered {(int)response.StatusCode}.");
            }

            await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken);
            using var json = await JsonDocument.ParseAsync(stream, cancellationToken: cancellationToken);
            var root = json.RootElement;
            DateTime? deployedAt = root.TryGetProperty("deployed_at", out var at) && at.ValueKind == JsonValueKind.Number
                ? DateTimeOffset.FromUnixTimeSeconds(at.GetInt64()).UtcDateTime
                : null;
            return ProbeResult.Ok(new VersionInfo(
                root.GetProperty("application").GetString() ?? "",
                root.GetProperty("version").GetString() ?? "",
                root.GetProperty("deployment").GetString() ?? "",
                deployedAt));
        }
        catch (Exception e) when (e is HttpRequestException or JsonException or KeyNotFoundException
                                      or InvalidOperationException ||
                                  (e is TaskCanceledException && !cancellationToken.IsCancellationRequested))
        {
            return ProbeResult.Fail(e is TaskCanceledException
                ? "The version endpoint did not answer in time."
                : $"The version endpoint could not be read: {e.Message}");
        }
    }
}
