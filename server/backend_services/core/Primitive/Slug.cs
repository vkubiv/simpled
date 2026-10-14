using System.Text;

namespace core.Primitive;

/// <summary>Organization and project slugs: lower-case letters, digits and single inner dashes.</summary>
public static class Slug
{
    public const int MaxLength = 40;

    /// <summary>The slug of <paramref name="name"/>; <paramref name="fallback"/> when nothing of the name survives.</summary>
    public static string From(string name, string fallback)
    {
        var sb = new StringBuilder();
#pragma warning disable CA1308 // Slugs are lower case by definition.
        foreach (var c in name.Trim().ToLowerInvariant())
#pragma warning restore CA1308
        {
            if (IsSlugChar(c))
            {
                sb.Append(c);
            }
            else if (sb.Length > 0 && sb[^1] != '-')
            {
                sb.Append('-');
            }
        }

        var slug = sb.ToString().Trim('-');
        if (slug.Length > MaxLength)
        {
            slug = slug[..MaxLength].Trim('-');
        }

        return slug.Length == 0 ? fallback : slug;
    }

    public static bool IsValid(string slug) =>
        slug.Length is > 0 and <= MaxLength &&
        slug.All(c => IsSlugChar(c) || c == '-') &&
        slug[0] != '-' && slug[^1] != '-' &&
        !slug.Contains("--", StringComparison.Ordinal);

    /// <summary><paramref name="slug"/>, or the first of slug-2, slug-3, … not in <paramref name="taken"/>.</summary>
    public static string FirstFree(string slug, IReadOnlyCollection<string> taken)
    {
        if (!taken.Contains(slug))
        {
            return slug;
        }

        for (var i = 2;; i++)
        {
            var candidate = $"{slug}-{i}";
            if (!taken.Contains(candidate))
            {
                return candidate;
            }
        }
    }

    private static bool IsSlugChar(char c) => c is >= 'a' and <= 'z' or >= '0' and <= '9';
}
