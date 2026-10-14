namespace config;

[System.Diagnostics.CodeAnalysis.ExcludeFromCodeCoverage]
public static class ConfigurationLoader
{
    public static CommonConfiguration LoadDefault()
    {
        if (Environment.GetEnvironmentVariable("DOCKERIZED") != "true")
        {
            DotNetEnv.Env.LoadMulti(new[]
            {
                ".env",
                "undockerized.env",
            });
        }

        var isDockerized = Environment.GetEnvironmentVariable("DOCKERIZED") == "true";
        var isDev = Environment.GetEnvironmentVariable("DEV_ENV") == "true";

        var dbPassword = LoadSecret("db_password");
        var defaultConnectionString = LoadRequiredEnv("DB_CONNECTION_STRING")
            .Replace("{password}", dbPassword, StringComparison.InvariantCulture);

        var webAppUrl = LoadRequiredEnv("WEB_APP_URL");

        return new CommonConfiguration
        {
            IsDockerized = isDockerized,
            IsDevelopmentEnvironment = isDev,

            DefaultConnectionString = defaultConnectionString,
            DbPassword = dbPassword,

            SendGridApiKey = LoadSecret("sendgrid_apikey"),
            SendGridApiHost = ResolveOptionalHost("SEND_GRID_API_HOST"),
            SendGridMockApiKey = LoadRequiredEnv("SEND_GRID_MOCK_API_KEY"),
            SendGridMockApiHost = ResolveOptionalHost("SEND_GRID_MOCK_API_HOST"),
            PostMasterEmail = LoadRequiredEnv("POSTMASTER_EMAIL"),

            WebAppUrl = webAppUrl.EndsWith('/') ? webAppUrl : webAppUrl + "/",

            SentryDsn = LoadOptionalEnv("SENTRY_DSN"),
            CorsOrigins = (LoadOptionalEnv("CORS_ORIGINS") ?? string.Empty)
                .Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries),
            AuthRateLimitPerMinute = int.Parse(LoadOptionalEnv("AUTH_RATE_LIMIT_PER_MINUTE") ?? "20",
                System.Globalization.CultureInfo.InvariantCulture),

            GitHubApiUrl = (LoadOptionalEnv("GITHUB_API_URL") ?? "https://api.github.com").TrimEnd('/'),
            GitHubTokenKey = LoadSecret("github_token_key"),
            ProbeProxyUrl = LoadOptionalEnv("PROBE_PROXY_URL"),
            MonitorInterval = TimeSpan.FromSeconds(LoadOptionalSeconds("MONITOR_INTERVAL_SECONDS", 60)),
            RepositorySyncInterval =
                TimeSpan.FromSeconds(LoadOptionalSeconds("REPOSITORY_SYNC_INTERVAL_SECONDS", 600)),
        };
    }

    private static int LoadOptionalSeconds(string name, int defaultSeconds) =>
        int.Parse(LoadOptionalEnv(name) ?? defaultSeconds.ToString(System.Globalization.CultureInfo.InvariantCulture),
            System.Globalization.CultureInfo.InvariantCulture);

    private static string LoadRequiredEnv(string name)
    {
        var value = Environment.GetEnvironmentVariable(name);
        if (string.IsNullOrEmpty(value))
        {
            throw new InvalidOperationException($"{name} env variable is not set");
        }

        return value;
    }

    private static string? LoadOptionalEnv(string name)
    {
        var value = Environment.GetEnvironmentVariable(name);
        return string.IsNullOrEmpty(value) ? null : value;
    }

    private static string? ResolveOptionalHost(string variableName)
    {
        var value = LoadRequiredEnv(variableName);
        return value == "not-set" ? null : value;
    }

    private static string SecretsBasePath =>
        Environment.GetEnvironmentVariable("DOCKERIZED") != "true" ? "./secrets" : "/secrets";

    private static string LoadSecret(string fileName)
    {
        var path = Path.Combine(SecretsBasePath, fileName);
        if (!File.Exists(path))
        {
            throw new InvalidOperationException($"Secret file {fileName} not found");
        }

        return File.ReadAllText(path).Trim();
    }
}
