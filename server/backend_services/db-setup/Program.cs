using System.Diagnostics.CodeAnalysis;
using config;
using db_setup;
using Microsoft.Extensions.Logging;

var config = ConfigurationLoader.LoadDefault();

using var loggerFactory = LoggerFactory.Create(builder =>
{
    builder
        .AddFilter("Microsoft", LogLevel.Warning)
        .AddFilter("System", LogLevel.Warning)
        .AddFilter("Microsoft.EntityFrameworkCore", LogLevel.Information)
        .AddConsole();

    if (!string.IsNullOrEmpty(config.SentryDsn))
    {
        builder.AddSentry(o =>
        {
            o.MinimumEventLevel = LogLevel.Warning;
            o.Dsn = config.SentryDsn;
        });
    }
});

const int attempts = 10;

// Applies pending migrations, retrying until the database accepts connections.
using var dbContext = DbConnect.TryToConnectToDb(config.DefaultConnectionString, attempts, loggerFactory);

[ExcludeFromCodeCoverage]
internal partial class Program
{
}
