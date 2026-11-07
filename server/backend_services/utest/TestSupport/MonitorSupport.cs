using core.Libs.EntityAccess;
using core.Operations;
using core.Services.GitHub;
using db.Entities;
using primary_db;
using utest.TestSupport.TestDoubles;

namespace utest.TestSupport;

public static class MonitorSupport
{
    public const string Token = "ghp_test";

    public static readonly TokenCipher Cipher = new("test-key");

    /// <summary>prod and staging of the `shop` application, as a deployment repository would hold them.</summary>
    public const string EnvSpec = """
        type: docker
        gateway:
          name: shop
          hosts:
            web: [shop.example.com, www.shop.example.com]
            api: api.shop.example.com
            ops: ops.example.com
        controller:
          host: ops
        deployments:
          prod:
            primary_host: web
            application:
              name: shop
              version: ^1.2.0
          canary:
            extends: prod
        """;

    public const string AppSpec = """
        name: shop
        version: 1.2.3
        app_services: {}
        """;

    public static FakeGitHub GitHub() => new FakeGitHub().AddToken(Token, "octocat");

    /// <summary>acme/shop with deployment/prod/envspec.yaml and app/appspec.yaml, readable with <see cref="Token"/>.</summary>
    public static FakeGitHub.Repository AddShop(FakeGitHub github) =>
        github.AddRepository("acme", "shop", Token, new Dictionary<string, string>
        {
            ["deployment/prod/envspec.yaml"] = EnvSpec,
            ["app/appspec.yaml"] = AppSpec,
            ["README.md"] = "# shop",
        });

    public static async Task<GitHubConnectionEntity> CreateConnection(PrimaryDbContext db,
        OrganizationEntity organization, string name = "acme-bot", string token = Token)
    {
        var connection = await GitHubConnectionOps.Create(organization.Id, name, token, new GitHubUser("octocat"),
            DateTime.UtcNow, Cipher, new Mutate<GitHubConnectionEntity>(db));
        await db.SaveChangesAsync();
        return connection;
    }

    /// <summary>A linked and synced repository.</summary>
    public static async Task<RepositoryEntity> LinkRepository(PrimaryDbContext db, ProjectEntity project,
        GitHubConnectionEntity connection, IGitHub github, string owner = "acme", string name = "shop")
    {
        var gitHubRepository = (await github.GetRepository(Token, owner, name, CancellationToken.None)).Value!;
        var repository = await RepositoryOps.Create(project.Id, connection, gitHubRepository, DateTime.UtcNow,
            new Mutate<RepositoryEntity>(db));
        await RepositoryOps.Sync(repository, Token, github, DateTime.UtcNow, new Mutate<DeployEnvironmentEntity>(db),
            new Mutate<DeploymentEntity>(db), CancellationToken.None);
        await db.SaveChangesAsync();
        return repository;
    }

    public static DeploymentEntity Deployment(PrimaryDbContext db, string name) =>
        db.Deployments.Single(d => d.Name == name);
}
