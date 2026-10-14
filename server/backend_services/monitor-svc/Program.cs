using System.Diagnostics.CodeAnalysis;
using config;
using core.Libs.EntityAccess;
using core.Services;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using monitor_svc;
using primary_db;

var config = ConfigurationLoader.LoadDefault();

var builder = WebApplication.CreateBuilder(args);
builder.Logging
    .AddFilter("Microsoft", LogLevel.Warning)
    .AddFilter("System", LogLevel.Warning);
if (!string.IsNullOrEmpty(config.SentryDsn))
{
    builder.Logging.AddSentry(o =>
    {
        o.MinimumEventLevel = LogLevel.Warning;
        o.Dsn = config.SentryDsn;
    });
}

builder.Services.AddDbContext<DbContext, PrimaryDbContext>(options =>
{
    options.UseNpgsql(config.DefaultConnectionString);
});
builder.Services.AddScoped(typeof(Read<>));
builder.Services.AddScoped(typeof(Mutate<>));
builder.Services.AddScoped<UnitOfWork>();
builder.Services.AddScoped<Clock>();

builder.Services.AddSingleton(config);
builder.Services.AddGitHub(config);
builder.Services.AddVersionProbe(config);
builder.Services.AddSingleton<MonitorHeartbeat>();
builder.Services.AddHostedService<MonitorWorker>();

var app = builder.Build();

// The only endpoint: unhealthy once no round has ended for a while, a loop stuck on a hung call, say.
var tolerance = MonitorWorker.Tick(config) * 4 + TimeSpan.FromMinutes(1);
app.MapGet("/health", (MonitorHeartbeat heartbeat) => heartbeat.IsAlive(DateTime.UtcNow, tolerance)
    ? Results.Ok("ok")
    : Results.StatusCode(StatusCodes.Status503ServiceUnavailable));

await app.RunAsync();

[ExcludeFromCodeCoverage]
internal partial class Program
{
}
