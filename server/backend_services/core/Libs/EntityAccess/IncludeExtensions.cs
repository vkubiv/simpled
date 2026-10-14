using System.Linq.Expressions;
using Microsoft.EntityFrameworkCore;
using Microsoft.EntityFrameworkCore.Query;

namespace core.Libs.EntityAccess;

/// <summary>
/// Include/ThenInclude that take the <see cref="Read{TEntity}"/> of the joined entity, so a query only reaches
/// tables its caller was given. EF's own overloads are banned in BannedSymbols.txt.
/// </summary>
#pragma warning disable RS0030
public static class IncludeExtensions
{
    public static IIncludableQueryable<TEntity, TProperty?> Include<TEntity, TProperty>(
        this IQueryable<TEntity> source,
        Expression<Func<TEntity, TProperty?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TProperty : class
        => EntityFrameworkQueryableExtensions.Include(source, SingleStep(navigation));

    public static IIncludableQueryable<TEntity, ICollection<TProperty>?> Include<TEntity, TProperty>(
        this IQueryable<TEntity> source,
        Expression<Func<TEntity, ICollection<TProperty>?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TProperty : class
        => EntityFrameworkQueryableExtensions.Include(source, SingleStep(navigation));

    public static IIncludableQueryable<TEntity, TProperty?> ThenInclude<TEntity, TPrevious, TProperty>(
        this IIncludableQueryable<TEntity, TPrevious?> source,
        Expression<Func<TPrevious, TProperty?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TPrevious : class where TProperty : class
        => EntityFrameworkQueryableExtensions.ThenInclude((IIncludableQueryable<TEntity, TPrevious>)source,
            SingleStep(navigation));

    public static IIncludableQueryable<TEntity, ICollection<TProperty>?> ThenInclude<TEntity, TPrevious, TProperty>(
        this IIncludableQueryable<TEntity, TPrevious?> source,
        Expression<Func<TPrevious, ICollection<TProperty>?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TPrevious : class where TProperty : class
        => EntityFrameworkQueryableExtensions.ThenInclude((IIncludableQueryable<TEntity, TPrevious>)source,
            SingleStep(navigation));

    public static IIncludableQueryable<TEntity, TProperty?> ThenInclude<TEntity, TPrevious, TProperty>(
        this IIncludableQueryable<TEntity, ICollection<TPrevious>?> source,
        Expression<Func<TPrevious, TProperty?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TPrevious : class where TProperty : class
        => EntityFrameworkQueryableExtensions.ThenInclude(source!, SingleStep(navigation));

    public static IIncludableQueryable<TEntity, ICollection<TProperty>?> ThenInclude<TEntity, TPrevious, TProperty>(
        this IIncludableQueryable<TEntity, ICollection<TPrevious>?> source,
        Expression<Func<TPrevious, ICollection<TProperty>?>> navigation,
        Read<TProperty> read)
        where TEntity : class where TPrevious : class where TProperty : class
        => EntityFrameworkQueryableExtensions.ThenInclude(source!, SingleStep(navigation));

    /// <summary>A path like <c>x => x.A.B</c> would reach B without its Read, so each step is its own call.</summary>
    private static Expression<Func<T, TResult>> SingleStep<T, TResult>(Expression<Func<T, TResult>> navigation)
    {
        if (navigation.Body is MemberExpression { Expression: ParameterExpression })
            return navigation;
        throw new ArgumentException($"Include one navigation per call, got {navigation}", nameof(navigation));
    }
}
