using config;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Design;

namespace primary_db;

/// <summary>
/// Used by `dotnet ef` (see `just add-migration`). Configuration comes from `.env` + `secrets/` in the current
/// directory; when primary-db has none, it falls back to the ones `simpled` writes into main-svc
/// (its `working_dir` in localenv.yaml), so `just infra` or `just prepare` is enough before migrating.
/// </summary>
public class DesignTimeDbContextFactory : IDesignTimeDbContextFactory<PrimaryDbContext>
{
    public PrimaryDbContext CreateDbContext(string[] args)
    {
        if (!File.Exists(".env") && File.Exists("../main-svc/.env"))
        {
            Directory.SetCurrentDirectory("../main-svc");
        }

        CommonConfiguration config = ConfigurationLoader.LoadDefault();

        var builder = new DbContextOptionsBuilder<PrimaryDbContext>();
        builder.UseNpgsql(config.DefaultConnectionString);
        return new PrimaryDbContext(builder.Options);
    }
}
