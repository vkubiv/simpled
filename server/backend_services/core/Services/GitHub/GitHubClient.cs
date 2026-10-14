using System.Net;
using System.Net.Http.Headers;
using System.Text.Json;

namespace core.Services.GitHub;

public sealed class GitHubClient : IGitHub
{
    private readonly HttpClient _http;
    private readonly string _apiUrl;

    /// <param name="apiUrl">https://api.github.com, without a trailing slash.</param>
    public GitHubClient(HttpClient http, string apiUrl)
    {
        _http = http;
        _apiUrl = apiUrl;
    }

    public Task<GitHubResult<GitHubUser>> GetUser(string token, CancellationToken cancellationToken) =>
        GetJson(token, "/user", json => new GitHubUser(json.GetProperty("login").GetString()!), cancellationToken);

    public Task<GitHubResult<GitHubRepository>> GetRepository(string token, string owner, string name,
        CancellationToken cancellationToken) =>
        GetJson(token, $"/repos/{Escape(owner)}/{Escape(name)}", json => new GitHubRepository(
                json.GetProperty("owner").GetProperty("login").GetString()!,
                json.GetProperty("name").GetString()!,
                json.GetProperty("default_branch").GetString()!),
            cancellationToken);

    public Task<GitHubResult<GitHubTree>> GetTree(string token, string owner, string name, string gitRef,
        CancellationToken cancellationToken) =>
        GetJson(token, $"/repos/{Escape(owner)}/{Escape(name)}/git/trees/{Escape(gitRef)}?recursive=1",
            json => new GitHubTree(
                json.GetProperty("sha").GetString()!,
                json.GetProperty("tree").EnumerateArray()
                    .Where(e => e.GetProperty("type").GetString() == "blob")
                    .Select(e => e.GetProperty("path").GetString()!)
                    .ToList(),
                json.TryGetProperty("truncated", out var truncated) && truncated.GetBoolean()),
            cancellationToken);

    public async Task<GitHubResult<string>> GetFile(string token, string owner, string name, string path,
        string gitRef, CancellationToken cancellationToken)
    {
        var escapedPath = string.Join('/', path.Split('/').Select(Escape));
        using var request = Request(token,
            $"/repos/{Escape(owner)}/{Escape(name)}/contents/{escapedPath}?ref={Escape(gitRef)}");
        request.Headers.Accept.Clear();
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github.raw+json"));
        return await Send(request, async response =>
            await response.Content.ReadAsStringAsync(cancellationToken), cancellationToken);
    }

    private async Task<GitHubResult<T>> GetJson<T>(string token, string path, Func<JsonElement, T> read,
        CancellationToken cancellationToken)
    {
        using var request = Request(token, path);
        return await Send(request, async response =>
        {
            await using var stream = await response.Content.ReadAsStreamAsync(cancellationToken);
            using var json = await JsonDocument.ParseAsync(stream, cancellationToken: cancellationToken);
            return read(json.RootElement);
        }, cancellationToken);
    }

    private HttpRequestMessage Request(string token, string path)
    {
        var request = new HttpRequestMessage(HttpMethod.Get, new Uri(_apiUrl + path));
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        request.Headers.Accept.Add(new MediaTypeWithQualityHeaderValue("application/vnd.github+json"));
        request.Headers.UserAgent.Add(new ProductInfoHeaderValue("simpled-server", "1"));
        request.Headers.Add("X-GitHub-Api-Version", "2022-11-28");
        return request;
    }

    private async Task<GitHubResult<T>> Send<T>(HttpRequestMessage request,
        Func<HttpResponseMessage, Task<T>> read, CancellationToken cancellationToken)
    {
        try
        {
            using var response = await _http.SendAsync(request, cancellationToken);
            return response.StatusCode switch
            {
                HttpStatusCode.OK => GitHubResult.Ok(await read(response)),
                HttpStatusCode.Unauthorized => GitHubResult.Fail<T>(GitHubFailure.Unauthorized,
                    "GitHub refused the token."),
                // 403 is also what a fine-grained token without the repository's permission gets.
                HttpStatusCode.NotFound or HttpStatusCode.Forbidden => GitHubResult.Fail<T>(GitHubFailure.NotFound,
                    $"GitHub answered {(int)response.StatusCode} for {request.RequestUri!.AbsolutePath}."),
                _ => GitHubResult.Fail<T>(GitHubFailure.Unavailable,
                    $"GitHub answered {(int)response.StatusCode} for {request.RequestUri!.AbsolutePath}."),
            };
        }
        catch (Exception e) when (e is HttpRequestException or JsonException or KeyNotFoundException
                                      or InvalidOperationException ||
                                  (e is TaskCanceledException && !cancellationToken.IsCancellationRequested))
        {
            return GitHubResult.Fail<T>(GitHubFailure.Unavailable, $"GitHub could not be read: {e.Message}");
        }
    }

    private static string Escape(string segment) => Uri.EscapeDataString(segment);
}
