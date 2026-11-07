using config;
using core.Services.GitHub;
using core.Services.Monitor;
using Microsoft.Extensions.DependencyInjection;

namespace core.Services;

public static class MonitoringServices
{
    /// <summary>GitHub access and the token cipher, for main-svc and monitor-svc alike.</summary>
    public static IServiceCollection AddGitHub(this IServiceCollection services, CommonConfiguration config)
    {
        services.AddSingleton(new TokenCipher(config.GitHubTokenKey));
        services.AddSingleton<IGitHub>(new GitHubClient(new HttpClient { Timeout = TimeSpan.FromSeconds(20) },
            config.GitHubApiUrl));
        return services;
    }

    public static IServiceCollection AddDeploymentProbe(this IServiceCollection services, CommonConfiguration config)
    {
        services.AddSingleton<IDeploymentProbe>(
            new DeploymentProbe(DeploymentProbe.CreateHttpClient(config.ProbeProxyUrl)));
        return services;
    }
}
