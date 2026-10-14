namespace core.Services.GitHub;

public enum GitHubFailure
{
    /// <summary>GitHub refused the token: revoked, expired or mistyped.</summary>
    Unauthorized,

    /// <summary>No such repository or file, or the token cannot see it; GitHub does not tell the two apart.</summary>
    NotFound,

    /// <summary>GitHub could not be reached or answered something unexpected.</summary>
    Unavailable,
}

/// <summary>A GitHub answer: <see cref="Value"/>, or why there is none.</summary>
public record GitHubResult<T>(T? Value, GitHubFailure? Failure, string? Message = null)
{
    public bool IsOk => Failure == null;
}

public static class GitHubResult
{
    public static GitHubResult<T> Ok<T>(T value) => new(value, null);

    public static GitHubResult<T> Fail<T>(GitHubFailure failure, string message) => new(default, failure, message);
}

public record GitHubUser(string Login);

public record GitHubRepository(string Owner, string Name, string DefaultBranch);

/// <param name="Sha">The tree's SHA, which changes whenever a file in it does.</param>
/// <param name="Truncated">GitHub stops listing very large trees; spec files past the cut are missed.</param>
public record GitHubTree(string Sha, IReadOnlyList<string> FilePaths, bool Truncated);

/// <summary>The part of GitHub's REST API the server uses, every call made with a caller-given token.</summary>
public interface IGitHub
{
    Task<GitHubResult<GitHubUser>> GetUser(string token, CancellationToken cancellationToken);

    Task<GitHubResult<GitHubRepository>> GetRepository(string token, string owner, string name,
        CancellationToken cancellationToken);

    /// <summary>Every file path of <paramref name="gitRef"/>, recursively.</summary>
    Task<GitHubResult<GitHubTree>> GetTree(string token, string owner, string name, string gitRef,
        CancellationToken cancellationToken);

    Task<GitHubResult<string>> GetFile(string token, string owner, string name, string path, string gitRef,
        CancellationToken cancellationToken);
}
