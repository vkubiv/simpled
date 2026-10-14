using YamlDotNet.Core;
using YamlDotNet.Serialization;

namespace core.Primitive;

public class SpecFormatException : Exception
{
    public SpecFormatException()
    {
    }

    public SpecFormatException(string message) : base(message)
    {
    }

    public SpecFormatException(string message, Exception innerException) : base(message, innerException)
    {
    }
}

/// <param name="Url">The primary host's base URL; null when the envspec names none or only a templated one.</param>
public record EnvSpecDeployment(string Name, string ApplicationName, string? VersionConstraint, string? Url);

/// <param name="Problems">Deployments that were skipped, and why.</param>
public record EnvSpec(IReadOnlyList<EnvSpecDeployment> Deployments, IReadOnlyList<string> Problems);

public record AppSpec(string Name, string? Version);

/// <summary>
/// The parts of simpled's envspec.yaml and appspec.yaml the monitor needs (docs/reference.md in the simpled
/// repository has the full formats). Anything else in the files is ignored, so a newer simpled's additions
/// never break a sync.
/// </summary>
public static class SpecFiles
{
    public static readonly string[] EnvSpecNames = ["envspec.yaml", "envspec.yml"];
    public static readonly string[] AppSpecNames = ["appspec.yaml", "appspec.yml"];

    public static bool IsEnvSpec(string path) => EnvSpecNames.Contains(FileName(path));

    public static bool IsAppSpec(string path) => AppSpecNames.Contains(FileName(path));

    /// <summary>`deployment/prod/envspec.yaml` is "prod"; an envspec at the root is named after the repository.</summary>
    public static string EnvironmentName(string path, string repositoryName)
    {
        var slash = path.LastIndexOf('/');
        if (slash < 0)
        {
            return repositoryName;
        }

        var directory = path[..slash];
        return directory[(directory.LastIndexOf('/') + 1)..];
    }

    public static EnvSpec ReadEnvSpec(string yaml)
    {
        var root = Map(Parse(yaml), "the file");
        var gateway = OptionalMap(root, "gateway");
        var tls = gateway == null ? null : OptionalMap(gateway, "tls");
        var scheme = tls != null && Scalar(tls, "disable") is "true" or "True" ? "http" : "https";
        var hosts = gateway == null ? null : OptionalMap(gateway, "hosts");

        var deployments = OptionalMap(root, "deployments") ??
                          throw new SpecFormatException("It has no `deployments`.");
        var result = new List<EnvSpecDeployment>();
        var problems = new List<string>();
        foreach (var (key, _) in deployments)
        {
            var name = key.ToString()!;
            var primaryHost = Inherited(deployments, name, d => Scalar(d, "primary_host"));
            var application = Inherited(deployments, name, d => OptionalMap(d, "application"));
            var applicationName = application == null ? null : Scalar(application, "name");
            if (applicationName == null)
            {
                problems.Add($"Deployment `{name}` names no application.");
                continue;
            }

            result.Add(new EnvSpecDeployment(name, applicationName, Scalar(application!, "version"),
                BaseUrl(hosts, primaryHost, scheme)));
        }

        return new EnvSpec(result, problems);
    }

    public static AppSpec ReadAppSpec(string yaml)
    {
        var root = Map(Parse(yaml), "the file");
        return new AppSpec(
            Scalar(root, "name") ?? throw new SpecFormatException("It has no `name`."),
            Scalar(root, "version"));
    }

    private static object? Parse(string yaml)
    {
        try
        {
            return new DeserializerBuilder().Build().Deserialize<object>(yaml);
        }
        catch (YamlException e)
        {
            throw new SpecFormatException($"It is not valid YAML: {e.Message}");
        }
    }

    /// <summary>The first host of <paramref name="primaryHost"/>, or of the only host when there is one.</summary>
    private static string? BaseUrl(Dictionary<object, object>? hosts, string? primaryHost, string scheme)
    {
        if (hosts == null)
        {
            return null;
        }

        var key = primaryHost ?? (hosts.Count == 1 ? hosts.Keys.First().ToString() : null);
        if (key == null || !hosts.TryGetValue(key, out var value))
        {
            return null;
        }

        var domain = value switch
        {
            string single => single,
            List<object> list => list.FirstOrDefault()?.ToString(),
            _ => null,
        };
        // `$port(...)` and other templates only resolve inside simpled.
        return string.IsNullOrWhiteSpace(domain) || domain.Contains('$', StringComparison.Ordinal)
            ? null
            : $"{scheme}://{domain.Trim()}";
    }

    /// <summary>A deployment's value, or that of the deployment it `extends`, transitively.</summary>
    private static T? Inherited<T>(Dictionary<object, object> deployments, string name,
        Func<Dictionary<object, object>, T?> read) where T : class
    {
        var seen = new HashSet<string>();
        var current = name;
        while (current != null && seen.Add(current) &&
               deployments.TryGetValue(current, out var value) && value is Dictionary<object, object> deployment)
        {
            var found = read(deployment);
            if (found != null)
            {
                return found;
            }

            current = Scalar(deployment, "extends");
        }

        return null;
    }

    private static Dictionary<object, object> Map(object? node, string what) =>
        node as Dictionary<object, object> ?? throw new SpecFormatException($"Expected a mapping in {what}.");

    private static Dictionary<object, object>? OptionalMap(Dictionary<object, object> map, string key) =>
        map.TryGetValue(key, out var value) ? value as Dictionary<object, object> : null;

    private static string? Scalar(Dictionary<object, object> map, string key) =>
        map.TryGetValue(key, out var value) && value is string s && s.Length > 0 ? s : null;

    private static string FileName(string path) => path[(path.LastIndexOf('/') + 1)..];
}
