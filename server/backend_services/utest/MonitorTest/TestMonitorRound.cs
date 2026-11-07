using core.Libs.EntityAccess;
using db.Entities;
using monitor_svc;
using utest.TestSupport;
using utest.TestSupport.TestDoubles;

namespace utest.MonitorTest;

public class TestMonitorRound
{
    [Fact]
    public async Task TestCheckDeployments_ProbesDueOnesAndSaves()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var project = await OrgSupport.CreateProject(db, organization, "Shop");
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub();
        MonitorSupport.AddShop(github);
        await MonitorSupport.LinkRepository(db, project, connection, github);
        var probe = new FakeDeploymentProbe();
        probe.Answer("https://ops.example.com/simpled/version/prod", "shop", "prod", "1.2.3", health: "healthy");
        var now = DateTime.UtcNow;

        var checkedCount = await MonitorRound.CheckDeployments(now, TimeSpan.FromMinutes(1), probe,
            new Mutate<DeploymentEntity>(db), new Mutate<DeploymentEventEntity>(db), new UnitOfWork(db),
            CancellationToken.None);

        Assert.Equal(2, checkedCount);
        Assert.Contains(("https://ops.example.com/simpled/version/prod", "https://ops.example.com/simpled/health/prod"),
            probe.Probed);
        Assert.Equal(DeploymentEntity.StatusUp, MonitorSupport.Deployment(db, "prod").Status);
        var canary = MonitorSupport.Deployment(db, "canary");
        Assert.Equal(DeploymentEntity.StatusUnknown, canary.Status);
        Assert.Equal(1, canary.FailureCount);

        Assert.Equal(0, await MonitorRound.CheckDeployments(now.AddSeconds(30), TimeSpan.FromMinutes(1), probe,
            new Mutate<DeploymentEntity>(db), new Mutate<DeploymentEventEntity>(db), new UnitOfWork(db),
            CancellationToken.None));
    }

    [Fact]
    public async Task TestSyncRepository_ReadsChangedSpecs()
    {
        var db = InMemoryPrimaryDb.Create();
        var (organization, _) = await OrgSupport.CreateOrg(db);
        var project = await OrgSupport.CreateProject(db, organization, "Shop");
        var connection = await MonitorSupport.CreateConnection(db, organization);
        var github = MonitorSupport.GitHub();
        var files = MonitorSupport.AddShop(github);
        var repository = await MonitorSupport.LinkRepository(db, project, connection, github);

        files.Files["app/appspec.yaml"] = MonitorSupport.AppSpec.Replace("1.2.3", "2.0.0", StringComparison.Ordinal);
        await MonitorRound.SyncRepository(repository.Id, DateTime.UtcNow, github, MonitorSupport.Cipher,
            new Read<RepositoryEntity>(db), new Read<GitHubConnectionEntity>(db),
            new Mutate<DeployEnvironmentEntity>(db), new Mutate<DeploymentEntity>(db), new UnitOfWork(db),
            CancellationToken.None);

        Assert.Equal("2.0.0", MonitorSupport.Deployment(db, "prod").RepoVersion);
    }

    [Fact]
    public void TestHeartbeat()
    {
        var heartbeat = new MonitorHeartbeat();
        var now = DateTime.UtcNow;

        Assert.False(heartbeat.IsAlive(now, TimeSpan.FromMinutes(1)));
        heartbeat.Beat(now);
        Assert.True(heartbeat.IsAlive(now.AddSeconds(30), TimeSpan.FromMinutes(1)));
        Assert.False(heartbeat.IsAlive(now.AddMinutes(2), TimeSpan.FromMinutes(1)));
    }
}
