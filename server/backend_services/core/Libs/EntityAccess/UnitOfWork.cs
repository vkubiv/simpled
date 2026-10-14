using System.Data;
using Microsoft.EntityFrameworkCore;

namespace core.Libs.EntityAccess;

public sealed class UnitOfWork
{
    private readonly DbContext _dbContext;

    public UnitOfWork(DbContext dbContext)
    {
        _dbContext = dbContext;
    }

    public async Task<int> SaveChangesAsync(CancellationToken cancellationToken = default)
        => await SaveChangesAsync(acceptAllChangesOnSuccess: true, cancellationToken);

    public async Task<int> SaveChangesAsync(
        bool acceptAllChangesOnSuccess,
        CancellationToken cancellationToken = default) =>
        await _dbContext.SaveChangesAsync(acceptAllChangesOnSuccess, cancellationToken);

    /// <summary>
    /// Runs <paramref name="body"/> inside an explicit database transaction and commits it.
    /// Ops that move units or money (ledger posting, order placement, subscription close, distributions)
    /// own their invariant, so they open the transaction themselves with this method; the controller still
    /// calls <see cref="SaveChangesAsync(CancellationToken)"/> once at the end for everything else.
    /// If a transaction is already open (nested call, or a test that wraps the whole case), the body just
    /// joins it. Providers that do not support transactions (EF InMemory) run the body directly.
    /// </summary>
    public async Task ExecuteInTransaction(
        Func<Task> body,
        IsolationLevel isolationLevel = IsolationLevel.RepeatableRead,
        CancellationToken cancellationToken = default)
    {
        if (_dbContext.Database.CurrentTransaction != null || !_dbContext.Database.IsRelational())
        {
            await body();
            return;
        }

        await using var tx = await _dbContext.Database.BeginTransactionAsync(isolationLevel, cancellationToken);
        await body();
        await _dbContext.SaveChangesAsync(cancellationToken);
        await tx.CommitAsync(cancellationToken);
    }
}
