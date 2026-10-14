using Microsoft.EntityFrameworkCore;
using primary_db;

namespace utest.TestSupport;

public static class InMemoryPrimaryDb
{
    public static PrimaryDbContext Create()
    {
        var options = new DbContextOptionsBuilder<PrimaryDbContext>()
            .UseInMemoryDatabase(databaseName: Guid.NewGuid().ToString())
            .Options;

        return new PrimaryDbContext(options);
    }
}