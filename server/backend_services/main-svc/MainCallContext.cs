using core;
using core.Libs.EntityAccess;

namespace main_svc;

public class MainCallContext : CallContext
{
    public required UnitOfWork UnitOfWork { get; init; }
}
