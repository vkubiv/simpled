using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.ChangeTracking;

namespace core.Libs.EntityAccess;

public sealed class Mutate<TEntity> where TEntity : class
{
    private readonly DbContext _dbContext;
    private readonly DbSet<TEntity> _dbSet;

    public Read<TEntity> Read => new Read<TEntity>(_dbSet);

    public Mutate(DbContext dbContext)
    {
        _dbSet = dbContext.Set<TEntity>();
        _dbContext = dbContext;
    }

    public EntityEntry<TEntity> Add(TEntity entity)
        => _dbSet.Add(entity);

    public async ValueTask<EntityEntry<TEntity>> AddAsync(
        TEntity entity,
        CancellationToken cancellationToken = default)
        => await _dbSet.AddAsync(entity, cancellationToken);

    public EntityEntry<TEntity> Attach(TEntity entity)
        => _dbSet.Attach(entity);

    public EntityEntry<TEntity> Remove(TEntity entity)
        => _dbSet.Remove(entity);

    public EntityEntry<TEntity> Update(TEntity entity)
        => _dbSet.Update(entity);

    public void AddRange(params TEntity[] entities)
        => _dbSet.AddRange(entities);

    public async Task AddRangeAsync(params TEntity[] entities)
        => await _dbSet.AddRangeAsync(entities);

    public void AttachRange(params TEntity[] entities)
        => _dbSet.AttachRange(entities);

    public void RemoveRange(params TEntity[] entities)
        => _dbSet.RemoveRange(entities);

    public void UpdateRange(params TEntity[] entities)
        => _dbSet.UpdateRange(entities);

    public void AddRange(IEnumerable<TEntity> entities)
        => _dbSet.AddRange(entities);

    public async Task AddRangeAsync(
        IEnumerable<TEntity> entities,
        CancellationToken cancellationToken = default)
        => await _dbSet.AddRangeAsync(entities, cancellationToken);

    public void AttachRange(IEnumerable<TEntity> entities)
        => _dbSet.AttachRange(entities);

    public void RemoveRange(IEnumerable<TEntity> entities)
        => _dbSet.RemoveRange(entities);

    public void UpdateRange(IEnumerable<TEntity> entities)
        => _dbSet.UpdateRange(entities);
}