using core.Libs.EntityAccess;
using core.Primitive;
using core.Services.GitHub;
using db.Entities;
using Microsoft.EntityFrameworkCore;

namespace core.Operations;

public static class RepositoryOps
{
    public static Task<RepositoryEntity?> TryGet(int repositoryId, Read<RepositoryEntity> readRepositories) =>
        readRepositories.FirstOrDefaultAsync(r => r.Id == repositoryId);

    /// <summary>Compares the owner and name GitHub answered with, which keep GitHub's own casing.</summary>
    public static Task<bool> IsLinked(int projectId, string owner, string name,
        Read<RepositoryEntity> readRepositories) =>
        readRepositories.AnyAsync(r => r.ProjectId == projectId && r.Owner == owner && r.Name == name);

    public static async Task<RepositoryEntity> Create(
        int projectId,
        GitHubConnectionEntity connection,
        GitHubRepository gitHubRepository,
        DateTime now,
        Mutate<RepositoryEntity> mutateRepositories)
    {
        var repository = new RepositoryEntity
        {
            ProjectId = projectId,
            GitHubConnection = connection,
            Owner = gitHubRepository.Owner,
            Name = gitHubRepository.Name,
            DefaultBranch = gitHubRepository.DefaultBranch,
            CreatedAt = now,
        };
        await mutateRepositories.AddAsync(repository);
        return repository;
    }

    /// <summary>Removes the repository; the database cascades to its environments, deployments and events.</summary>
    public static void Delete(RepositoryEntity repository, Mutate<RepositoryEntity> mutateRepositories) =>
        mutateRepositories.Remove(repository);

    /// <summary>Repositories whose last sync is older than <paramref name="interval"/>, oldest first.</summary>
    public static Task<List<RepositoryEntity>> DueForSync(DateTime now, TimeSpan interval, int limit,
        Read<RepositoryEntity> readRepositories)
    {
        var before = now - interval;
        return readRepositories
            .Where(r => r.SyncedAt == null || r.SyncedAt <= before)
            .OrderBy(r => r.SyncedAt)
            .Take(limit)
            .ToListAsync();
    }

    /// <summary>
    /// Reads the default branch's envspecs and appspecs and makes the repository's environments and deployments
    /// match them. A deployment keeps its monitor state unless its address changed. An envspec that cannot be
    /// read leaves its environment as it was; every problem lands in <see cref="RepositoryEntity.SyncError"/>.
    /// </summary>
    public static async Task Sync(
        RepositoryEntity repository,
        string token,
        IGitHub github,
        DateTime now,
        Mutate<DeployEnvironmentEntity> mutateEnvironments,
        Mutate<DeploymentEntity> mutateDeployments,
        CancellationToken cancellationToken)
    {
        repository.SyncedAt = now;

        var gitHubRepository = await github.GetRepository(token, repository.Owner, repository.Name,
            cancellationToken);
        if (!gitHubRepository.IsOk)
        {
            repository.SyncError = gitHubRepository.Message;
            return;
        }

        repository.DefaultBranch = gitHubRepository.Value!.DefaultBranch;
        var tree = await github.GetTree(token, repository.Owner, repository.Name, repository.DefaultBranch,
            cancellationToken);
        if (!tree.IsOk)
        {
            repository.SyncError = tree.Message;
            return;
        }

        var problems = new List<string>();
        if (tree.Value!.Truncated)
        {
            problems.Add("GitHub listed only part of the repository; spec files past the cut were not read.");
        }

        var paths = tree.Value.FilePaths
            .Where(p => !p.Contains("node_modules/", StringComparison.Ordinal))
            .OrderBy(p => p, StringComparer.Ordinal)
            .ToList();

        var appVersions = new Dictionary<string, string?>();
        foreach (var path in paths.Where(SpecFiles.IsAppSpec))
        {
            var appSpec = await ReadSpec(github, token, repository, path, SpecFiles.ReadAppSpec, problems,
                cancellationToken);
            if (appSpec != null)
            {
                appVersions.TryAdd(appSpec.Name, appSpec.Version);
            }
        }

        var envSpecPaths = paths.Where(SpecFiles.IsEnvSpec).ToList();
        var envSpecs = new Dictionary<string, EnvSpec>();
        foreach (var path in envSpecPaths)
        {
            var envSpec = await ReadSpec(github, token, repository, path, SpecFiles.ReadEnvSpec, problems,
                cancellationToken);
            if (envSpec != null)
            {
                envSpecs[path] = envSpec;
                problems.AddRange(envSpec.Problems.Select(p => $"{path}: {p}"));
            }
        }

        await Reconcile(repository, envSpecPaths, envSpecs, appVersions, mutateEnvironments, mutateDeployments,
            cancellationToken);

        repository.SyncedRevision = tree.Value.Sha;
        repository.SyncError = problems.Count == 0 ? null : string.Join('\n', problems);
    }

    private static async Task<T?> ReadSpec<T>(IGitHub github, string token, RepositoryEntity repository,
        string path, Func<string, T> read, List<string> problems, CancellationToken cancellationToken)
        where T : class
    {
        var file = await github.GetFile(token, repository.Owner, repository.Name, path, repository.DefaultBranch,
            cancellationToken);
        if (!file.IsOk)
        {
            problems.Add($"{path}: {file.Message}");
            return null;
        }

        try
        {
            return read(file.Value!);
        }
        catch (SpecFormatException e)
        {
            problems.Add($"{path}: {e.Message}");
            return null;
        }
    }

    /// <param name="envSpecPaths">Every envspec in the repository, read or not.</param>
    /// <param name="envSpecs">The envspecs that could be read, by path.</param>
    private static async Task Reconcile(
        RepositoryEntity repository,
        List<string> envSpecPaths,
        Dictionary<string, EnvSpec> envSpecs,
        Dictionary<string, string?> appVersions,
        Mutate<DeployEnvironmentEntity> mutateEnvironments,
        Mutate<DeploymentEntity> mutateDeployments,
        CancellationToken cancellationToken)
    {
        var environments = await mutateEnvironments.Read
            .Where(e => e.RepositoryId == repository.Id)
            .ToListAsync(cancellationToken);
        var environmentIds = environments.Select(e => e.Id).ToList();
        var deployments = await mutateDeployments.Read
            .Where(d => environmentIds.Contains(d.EnvironmentId))
            .ToListAsync(cancellationToken);

        foreach (var environment in environments.Where(e => !envSpecPaths.Contains(e.Path)))
        {
            mutateDeployments.RemoveRange(deployments.Where(d => d.EnvironmentId == environment.Id));
            mutateEnvironments.Remove(environment);
        }

        foreach (var (path, envSpec) in envSpecs)
        {
            var environment = environments.FirstOrDefault(e => e.Path == path);
            if (environment == null)
            {
                environment = new DeployEnvironmentEntity
                {
                    Repository = repository,
                    Name = SpecFiles.EnvironmentName(path, repository.Name),
                    Path = path,
                };
                await mutateEnvironments.AddAsync(environment, cancellationToken);
            }

            var existing = deployments.Where(d => d.EnvironmentId == environment.Id).ToList();
            mutateDeployments.RemoveRange(existing.Where(d => envSpec.Deployments.All(s => s.Name != d.Name)));

            foreach (var spec in envSpec.Deployments)
            {
                var deployment = existing.FirstOrDefault(d => d.Name == spec.Name);
                if (deployment == null)
                {
                    deployment = new DeploymentEntity
                    {
                        Environment = environment,
                        Name = spec.Name,
                        ApplicationName = spec.ApplicationName,
                        Status = DeploymentEntity.StatusUnknown,
                    };
                    await mutateDeployments.AddAsync(deployment, cancellationToken);
                }

                deployment.ApplicationName = spec.ApplicationName;
                deployment.VersionConstraint = spec.VersionConstraint;
                deployment.RepoVersion = appVersions.GetValueOrDefault(spec.ApplicationName);
                deployment.Url = spec.Url;
                if (deployment.VersionUrl != spec.VersionUrl || deployment.HealthUrl != spec.HealthUrl)
                {
                    deployment.VersionUrl = spec.VersionUrl;
                    deployment.HealthUrl = spec.HealthUrl;
                    MonitorOps.Reset(deployment);
                }
            }
        }
    }
}
