using System.Collections;
using System.Linq.Expressions;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.ChangeTracking;

namespace core.Libs.EntityAccess;

#pragma warning disable CA1716
public class Read<TEntity> : IQueryable<TEntity> where TEntity : class
#pragma warning restore CA1716
{
    public Read(DbSet<TEntity> dbSet)
    {
        DbSet = dbSet;
        _queryableImplementation = dbSet;
    }

    [Microsoft.Extensions.DependencyInjection.ActivatorUtilitiesConstructor]
    public Read(DbContext dbContext) : this(dbContext.Set<TEntity>())
    {
    }

    public IEnumerator<TEntity> GetEnumerator()
    {
        return _queryableImplementation.GetEnumerator();
    }

    IEnumerator IEnumerable.GetEnumerator()
    {
        return ((IEnumerable)_queryableImplementation).GetEnumerator();
    }

    public Type ElementType => _queryableImplementation.ElementType;

    public Expression Expression => _queryableImplementation.Expression;

    public IQueryProvider Provider => _queryableImplementation.Provider;


    public LocalView<TEntity> Local
        => DbSet.Local;


    public TEntity? Find(params object?[]? keyValues)
        => DbSet.Find(keyValues);

    public async ValueTask<TEntity?> FindAsync(params object?[]? keyValues)
        => await DbSet.FindAsync(keyValues);


    public async ValueTask<TEntity?> FindAsync(object?[]? keyValues, CancellationToken cancellationToken)
        => await DbSet.FindAsync(keyValues, cancellationToken);


    private DbSet<TEntity> DbSet { get; }

    private readonly IQueryable<TEntity> _queryableImplementation;
}