using System.Threading.RateLimiting;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.HttpOverrides;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.DependencyInjection;

namespace core.Auth;

/// <summary>
/// Per client IP fixed window on the auth controllers (login, registration, token mails): slows password
/// guessing across accounts, which the per-account lockout alone does not.
/// </summary>
public static class AuthRateLimiting
{
    public const string Policy = "auth";

    public static IServiceCollection AddAuthRateLimiting(this IServiceCollection services, int permitsPerMinute)
    {
        // The services sit behind one gateway hop that appends the caller to X-Forwarded-For; the last entry
        // is the client. Without this every request would share the gateway's address and one bucket.
        services.Configure<ForwardedHeadersOptions>(options =>
        {
            options.ForwardedHeaders = ForwardedHeaders.XForwardedFor | ForwardedHeaders.XForwardedProto;
            options.ForwardLimit = 1;
            options.KnownIPNetworks.Clear();
            options.KnownProxies.Clear();
        });

        return services.AddRateLimiter(options =>
        {
            options.RejectionStatusCode = StatusCodes.Status429TooManyRequests;
            options.AddPolicy(Policy, context => RateLimitPartition.GetFixedWindowLimiter(
                context.Connection.RemoteIpAddress?.ToString() ?? "unknown",
                _ => new FixedWindowRateLimiterOptions
                {
                    PermitLimit = permitsPerMinute,
                    Window = TimeSpan.FromMinutes(1),
                    QueueLimit = 0,
                }));
        });
    }
}
