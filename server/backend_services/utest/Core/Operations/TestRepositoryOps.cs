using core.Libs.EntityAccess;
using core.Operations;
using db.Entities;
using primary_db;
using utest.TestSupport;
using utest.TestSupport.TestDoubles;

namespace utest.Core.Operations;

public class TestRepositoryOps
{
    private static async Task Sync(PrimaryDbContext db, RepositoryEntity repository, FakeGitHub github)
    {
        await RepositoryOps.Sync(repository, MonitorSupport.Token, github, DateTime.UtcNow,
            new Mutate<DeployEnvironmentEntity>(db), new Mutate<DeploymentEntity>(db), CancellationToken.None);
        await db.SaveChangesAsync();
    }

    private static async Task<(PrimaryDbContext Db, FakeGitHub GitHub, FakeGitHub.Repository Files,
        RepositoryEntity Repository)> Linked()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var project = await OrgSupport.CreateProject(db, organization, "Shop");
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub();
        var files = MonitorSupport.AddShop(github);
        var repository = await MonitorSupport.LinkRepository(db, project, connection, github);
        return (db, github, files, repository);
    }

    [Fact]
    public async Task TestSync_EnvironmentsAndDeploymentsFromSpecs()
    {
        var (db, _, _, repository) = await Linked();

        Assert.Null(repository.SyncError);
        Assert.NotNull(repository.SyncedRevision);
        var environment = Assert.Single(db.Environments);
        Assert.Equal("prod", environment.Name);
        Assert.Equal("deployment/prod/envspec.yaml", environment.Path);
        Assert.Equal(["canary", "prod"], db.Deployments.Select(d => d.Name).Order());
        var prod = MonitorSupport.Deployment(db, "prod");
        Assert.Equal("shop", prod.ApplicationName);
        Assert.Equal("1.2.3", prod.RepoVersion);
        Assert.Equal("https://shop.example.com", prod.Url);
        Assert.Equal(DeploymentEntity.StatusUnknown, prod.Status);
    }

    [Fact]
    public async Task TestSync_KeepsMonitorStateUnlessTheAddressMoves()
    {
        var (db, github, files, repository) = await Linked();
        var prod = MonitorSupport.Deployment(db, "prod");
        prod.Status = DeploymentEntity.StatusUp;
        prod.LiveVersion = "1.2.3";
        await db.SaveChangesAsync();

        files.Files["app/appspec.yaml"] = MonitorSupport.AppSpec.Replace("1.2.3", "1.3.0", StringComparison.Ordinal);
        await Sync(db, repository, github);
        Assert.Equal(DeploymentEntity.StatusUp, prod.Status);
        Assert.Equal("1.3.0", prod.RepoVersion);

        files.Files["deployment/prod/envspec.yaml"] =
            MonitorSupport.EnvSpec.Replace("[shop.example.com", "[new.shop.example.com", StringComparison.Ordinal);
        await Sync(db, repository, github);
        Assert.Equal("https://new.shop.example.com", prod.Url);
        Assert.Equal(DeploymentEntity.StatusUnknown, prod.Status);
        Assert.Null(prod.LiveVersion);
    }

    [Fact]
    public async Task TestSync_RemovedSpecsRemoveTheirRows()
    {
        var (db, github, files, repository) = await Linked();

        files.Files["deployment/prod/envspec.yaml"] = """
            deployments:
              prod:
                application: { name: shop }
            """;
        files.Files["deployment/dev/envspec.yaml"] = """
            deployments:
              dev:
                application: { name: shop }
            """;
        await Sync(db, repository, github);
        Assert.Equal(["dev", "prod"], db.Deployments.Select(d => d.Name).Order());

        files.Files.Remove("deployment/prod/envspec.yaml");
        await Sync(db, repository, github);

        Assert.Equal("dev", Assert.Single(db.Environments).Name);
        Assert.Equal("dev", Assert.Single(db.Deployments).Name);
    }

    [Fact]
    public async Task TestSync_UnreadableEnvSpecKeepsItsEnvironment()
    {
        var (db, github, files, repository) = await Linked();

        files.Files["deployment/prod/envspec.yaml"] = "deployments: [oops";
        await Sync(db, repository, github);

        Assert.Equal(2, db.Deployments.Count());
        Assert.Contains("deployment/prod/envspec.yaml", repository.SyncError, StringComparison.Ordinal);

        files.Files["deployment/prod/envspec.yaml"] = MonitorSupport.EnvSpec;
        files.Unreadable.Add("app/appspec.yaml");
        await Sync(db, repository, github);
        Assert.Contains("app/appspec.yaml", repository.SyncError, StringComparison.Ordinal);
        Assert.Null(MonitorSupport.Deployment(db, "prod").RepoVersion);
    }

    [Fact]
    public async Task TestSync_GitHubFailureChangesNothingButTheError()
    {
        var (db, github, _, repository) = await Linked();
        var revision = repository.SyncedRevision;

        github.Down = true;
        await Sync(db, repository, github);

        Assert.Contains("down", repository.SyncError, StringComparison.Ordinal);
        Assert.Equal(revision, repository.SyncedRevision);
        Assert.Equal(2, db.Deployments.Count());
    }
}
