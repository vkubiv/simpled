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
}
