using core;
using core.Libs.EntityAccess;
using core.Services;
using main_svc;
using Microsoft.EntityFrameworkCore;
using Microsoft.Extensions.Logging.Abstractions;

namespace utest.TestSupport;

public static class CallContextSupport
{
    public static CallContext New()
    {
        return new CallContext
        {
            CancellationToken = CancellationToken.None,
            Logger = NullLogger.Instance,
            Clock = new Clock(),
        };
    }

    public static MainCallContext Main(DbContext dbContext)
    {
        return new MainCallContext
        {
            CancellationToken = CancellationToken.None,
            Logger = NullLogger.Instance,
            Clock = new Clock(),
            UnitOfWork = new UnitOfWork(dbContext),
        };
    }
}
