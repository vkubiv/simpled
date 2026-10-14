using core.Services.GitHub;

namespace utest.TestSupport.TestDoubles;

/// <summary>GitHub with the tokens and repositories a test adds; files can change between syncs.</summary>
public class FakeGitHub : IGitHub
{
    public class Repository
    {
        public required string Owner { get; init; }
        public required string Name { get; init; }
        public string DefaultBranch { get; set; } = "main";
        public HashSet<string> Tokens { get; } = [];
        public Dictionary<string, string> Files { get; } = [];

        /// <summary>Files listed in the tree whose contents GitHub refuses to send.</summary>
        public HashSet<string> Unreadable { get; } = [];
    }

    private readonly Dictionary<string, string> _tokens = [];
    private readonly Dictionary<string, Repository> _repositories = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>Set to make every call fail as if GitHub were unreachable.</summary>
    public bool Down { get; set; }

    public FakeGitHub AddToken(string token, string login)
    {
        _tokens[token] = login;
        return this;
    }

    public Repository AddRepository(string owner, string name, string token, Dictionary<string, string>? files = null)
    {
        var repository = new Repository { Owner = owner, Name = name };
        repository.Tokens.Add(token);
        foreach (var (path, text) in files ?? [])
        {
            repository.Files[path] = text;
        }

        _repositories[$"{owner}/{name}"] = repository;
        return repository;
    }

    public Task<GitHubResult<GitHubUser>> GetUser(string token, CancellationToken cancellationToken) =>
        Task.FromResult(Check<GitHubUser>(token) ?? GitHubResult.Ok(new GitHubUser(_tokens[token])));

    public Task<GitHubResult<GitHubRepository>> GetRepository(string token, string owner, string name,
        CancellationToken cancellationToken)
    {
        var failure = Check<GitHubRepository>(token, owner, name);
        var repository = _repositories.GetValueOrDefault($"{owner}/{name}");
        return Task.FromResult(failure ??
                               GitHubResult.Ok(new GitHubRepository(repository!.Owner, repository.Name,
                                   repository.DefaultBranch)));
    }

    public Task<GitHubResult<GitHubTree>> GetTree(string token, string owner, string name, string gitRef,
        CancellationToken cancellationToken)
    {
        var failure = Check<GitHubTree>(token, owner, name);
        var repository = _repositories.GetValueOrDefault($"{owner}/{name}");
        return Task.FromResult(failure ?? GitHubResult.Ok(new GitHubTree(
            string.Join('|', repository!.Files.OrderBy(f => f.Key).Select(f => f.Key + "=" + f.Value)).GetHashCode()
                .ToString("x", System.Globalization.CultureInfo.InvariantCulture),
            repository.Files.Keys.ToList(),
            false)));
    }

    public Task<GitHubResult<string>> GetFile(string token, string owner, string name, string path, string gitRef,
        CancellationToken cancellationToken)
    {
        var failure = Check<string>(token, owner, name);
        var repository = _repositories.GetValueOrDefault($"{owner}/{name}");
        if (failure != null)
        {
            return Task.FromResult(failure);
        }

        return Task.FromResult(repository!.Files.TryGetValue(path, out var text) && !repository.Unreadable.Contains(path)
            ? GitHubResult.Ok(text)
            : GitHubResult.Fail<string>(GitHubFailure.NotFound, $"GitHub answered 404 for {path}."));
    }

    private GitHubResult<T>? Check<T>(string token, string? owner = null, string? name = null)
    {
        if (Down)
        {
            return GitHubResult.Fail<T>(GitHubFailure.Unavailable, "GitHub could not be read: down.");
        }

        if (!_tokens.ContainsKey(token))
        {
            return GitHubResult.Fail<T>(GitHubFailure.Unauthorized, "GitHub refused the token.");
        }

        if (owner != null &&
            (!_repositories.TryGetValue($"{owner}/{name}", out var repository) || !repository.Tokens.Contains(token)))
        {
            return GitHubResult.Fail<T>(GitHubFailure.NotFound, $"GitHub answered 404 for {owner}/{name}.");
        }

        return null;
    }
}
