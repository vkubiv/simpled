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
/// <param name="VersionUrl">Where the deployment's version is read; null when the envspec gives no reachable host.</param>
/// <param name="HealthUrl">Where its health is read: the controller's, on Docker since simpled 2.1.</param>
public record EnvSpecDeployment(
    string Name,
    string ApplicationName,
    string? VersionConstraint,
    string? Url,
    string? VersionUrl,
    string? HealthUrl);

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

    /// <summary>What simpled reads as an env spec, which a deployment's file therefore cannot be named.</summary>
    private static readonly string[] OwnNames = [.. EnvSpecNames, "localenv.yaml", "localenv.yml"];

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

    /// <summary>What every gateway host answers on Kubernetes and locally, and on Docker before simpled 2.1.</summary>
    public const string WellKnownVersionPath = "/.well-known/simpled/version";

    /// <summary>
    /// The files the envspec's deployments are written in (`prod: { file: prod.yaml }`), which simpled reads from
    /// the envspec's own directory only. The file names are checked as simpled checks them; what else simpled
    /// refuses about such an entry (other fields beside `file`, a file naming a file) is read leniently.
    /// </summary>
    public static IReadOnlyList<string> DeploymentFiles(string yaml)
    {
        var deployments = OptionalMap(Map(Parse(yaml), "the file"), "deployments");
        return deployments == null
            ? []
            : deployments.Values.OfType<Dictionary<object, object>>()
                .Select(d => Scalar(d, "file"))
                .OfType<string>()
                .Select(DeploymentFileName)
                .Distinct()
                .ToList();
    }

    /// <param name="deploymentFiles">The contents of <see cref="DeploymentFiles"/>, by name.</param>
    public static EnvSpec ReadEnvSpec(string yaml, IReadOnlyDictionary<string, string>? deploymentFiles = null)
    {
        var root = Map(Parse(yaml), "the file");
        var gateway = OptionalMap(root, "gateway");
        var tls = gateway == null ? null : OptionalMap(gateway, "tls");
        var scheme = tls != null && IsTrue(tls, "disable") ? "http" : "https";
        var hosts = gateway == null ? null : OptionalMap(gateway, "hosts");
        // Since 2.1 a Docker env spec names the controller's host, which answers for every deployment under
        // /simpled/; one without the block was prepared by an older simpled, whose hosts answer the well-known path.
        var controller = Scalar(root, "type") == "docker" ? OptionalMap(root, "controller") : null;
        var controllerHost = controller == null ? null : Scalar(controller, "host");
        var controllerUrl = controllerHost == null ? null : HostUrl(hosts, controllerHost, scheme);

        var deployments = OptionalMap(root, "deployments") ??
                          throw new SpecFormatException("It has no `deployments`.");
        foreach (var key in deployments.Keys.ToList())
        {
            if (deployments[key] is Dictionary<object, object> entry && Scalar(entry, "file") is { } file)
            {
                deployments[key] = ReadDeploymentFile(key.ToString()!, DeploymentFileName(file), deploymentFiles);
            }
        }

        var result = new List<EnvSpecDeployment>();
        var problems = new List<string>();
        foreach (var (key, value) in deployments)
        {
            // A template other deployments extend: simpled never deploys it.
            if (value is Dictionary<object, object> own && IsTrue(own, "abstract"))
            {
                continue;
            }

            var name = key.ToString()!;
            var primaryHost = Inherited(deployments, name, d => Scalar(d, "primary_host"));
            var application = Inherited(deployments, name, d => OptionalMap(d, "application"));
            var applicationName = application == null ? null : Scalar(application, "name");
            if (applicationName == null)
            {
                problems.Add($"Deployment `{name}` names no application.");
                continue;
            }

            var url = BaseUrl(hosts, primaryHost, scheme);
            var escaped = Uri.EscapeDataString(name);
            result.Add(controllerHost == null
                ? new EnvSpecDeployment(name, applicationName, Scalar(application!, "version"), url,
                    url == null ? null : url + WellKnownVersionPath, null)
                : new EnvSpecDeployment(name, applicationName, Scalar(application!, "version"), url,
                    controllerUrl == null ? null : $"{controllerUrl}/simpled/version/{escaped}",
                    controllerUrl == null ? null : $"{controllerUrl}/simpled/health/{escaped}"));
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

    private static string DeploymentFileName(string file)
    {
        var name = file.StartsWith("./", StringComparison.Ordinal) ? file[2..] : file;
        if (name is "" or "." or ".." || name.Contains('/', StringComparison.Ordinal) ||
            name.Contains('\\', StringComparison.Ordinal))
        {
            throw new SpecFormatException($"`file: {file}` must name a file in the envspec's own directory.");
        }

        if (!name.EndsWith(".yaml", StringComparison.OrdinalIgnoreCase) &&
            !name.EndsWith(".yml", StringComparison.OrdinalIgnoreCase))
        {
            throw new SpecFormatException($"`file: {file}` must be a .yaml or .yml file.");
        }

        if (OwnNames.Contains(name, StringComparer.OrdinalIgnoreCase))
        {
            throw new SpecFormatException($"`file: {file}` is an env spec's own name.");
        }

        return name;
    }

    private static Dictionary<object, object> ReadDeploymentFile(string deployment, string fileName,
        IReadOnlyDictionary<string, string>? deploymentFiles)
    {
        if (deploymentFiles == null || !deploymentFiles.TryGetValue(fileName, out var yaml))
        {
            throw new SpecFormatException($"Deployment `{deployment}` is written in {fileName}, which was not read.");
        }

        try
        {
            return Map(Parse(yaml), "the file");
        }
        catch (SpecFormatException e)
        {
            throw new SpecFormatException($"{fileName}: {e.Message}", e);
        }
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
        return key == null ? null : HostUrl(hosts, key, scheme);
    }

    /// <summary>The first domain of the host alias <paramref name="key"/>.</summary>
    private static string? HostUrl(Dictionary<object, object>? hosts, string key, string scheme)
    {
        if (hosts == null || !hosts.TryGetValue(key, out var value))
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

    /// <summary>`true` in any of the spellings simpled's YAML reads as one.</summary>
    private static bool IsTrue(Dictionary<object, object> map, string key) =>
        Scalar(map, key) is "true" or "True" or "TRUE";

    private static string FileName(string path) => path[(path.LastIndexOf('/') + 1)..];
}
