namespace config;

public record CommonConfiguration
{
    public required bool IsDockerized { get; init; }
    public required bool IsDevelopmentEnvironment { get; init; }

    public required string DefaultConnectionString { get; init; }
    public required string DbPassword { get; init; }

    public required string SendGridApiKey { get; init; }
    public required string? SendGridApiHost { get; init; }
    public required string SendGridMockApiKey { get; init; }
    public required string? SendGridMockApiHost { get; init; }

    public required string PostMasterEmail { get; init; }

    /// <summary>Base URL of the web app, with a trailing slash; links in emails are built on it.</summary>
    public required string WebAppUrl { get; init; }

    /// <summary>Empty means Sentry is disabled.</summary>
    public required string? SentryDsn { get; init; }

    /// <summary>Allowed browser origins when the web app is served from another origin (vite dev server).</summary>
    public required IReadOnlyList<string> CorsOrigins { get; init; }

    /// <summary>Requests per minute per client IP on the auth endpoints.</summary>
    public required int AuthRateLimitPerMinute { get; init; }

    /// <summary>GitHub's REST API; the e2e stack points it at simpled-fakes.</summary>
    public required string GitHubApiUrl { get; init; }

    /// <summary>Encrypts the GitHub tokens organizations save (TokenCipher).</summary>
    public required string GitHubTokenKey { get; init; }

    /// <summary>
    /// An HTTP proxy the monitor reaches deployments through. Local only: simpled-fakes plays every deployment's
    /// gateway, whatever its host name.
    /// </summary>
    public required string? ProbeProxyUrl { get; init; }

    /// <summary>How often monitor-svc checks each deployment's version endpoint.</summary>
    public required TimeSpan MonitorInterval { get; init; }

    /// <summary>How often monitor-svc re-reads each linked repository's spec files.</summary>
    public required TimeSpan RepositorySyncInterval { get; init; }
}
