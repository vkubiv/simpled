using System.Diagnostics.CodeAnalysis;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging;
using primary_db;

namespace db_setup;

[ExcludeFromCodeCoverage]
internal static  class DbConnect
{
    public static PrimaryDbContext TryToConnectToDb(string connectionString, int attempts,
        ILoggerFactory loggerFactory)
    {
        var logger = loggerFactory.CreateLogger<Program>();

        for (var i = 0; i < attempts; ++i)
        {
            try
            {
                if (i != 0)
                {
                    Thread.Sleep(5000);
                    logger.LogWarning("New attempt to connect to db.");
                }

                var dbOptions =
                    new DbContextOptionsBuilder<PrimaryDbContext>().UseNpgsql(connectionString).UseLoggerFactory(loggerFactory);


                var dbContext = new PrimaryDbContext(dbOptions.Options);

                dbContext.Database.Migrate();
                logger.LogInformation("Success.");

                return dbContext;
            }
            catch (Exception e)
            {
                logger.LogWarning("DB connection failed.");

                if (i == (attempts - 1))
                {
                    logger.LogCritical(e, "Migration failed.");
                    throw;
                }
            }
        }

        throw new InvalidOperationException($"DB connection failed after {attempts} attempts.");
    }
}