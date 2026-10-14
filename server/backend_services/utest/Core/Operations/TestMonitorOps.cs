using core.Libs.EntityAccess;
using core.Operations;
using core.Services.Monitor;
using db.Entities;
using primary_db;
using utest.TestSupport;

namespace utest.Core.Operations;

public class TestMonitorOps
{
    private static readonly DateTime Now = new(2026, 10, 14, 12, 0, 0, DateTimeKind.Utc);

    private static DeploymentEntity NewDeployment() => new()
    {
        Name = "prod",
        ApplicationName = "shop",
        Url = "https://shop.example.com",
        Status = DeploymentEntity.StatusUnknown,
    };

    private static ProbeResult Answer(string version, string application = "shop", string deployment = "prod") =>
        ProbeResult.Ok(new VersionInfo(application, version, deployment, Now.AddHours(-1)));

    private static async Task Apply(PrimaryDbContext db, DeploymentEntity deployment, ProbeResult result,
        int minutes = 0)
    {
        await MonitorOps.ApplyCheck(deployment, result, Now.AddMinutes(minutes),
            new Mutate<DeploymentEventEntity>(db));
        await db.SaveChangesAsync();
    }

    private static async Task<(PrimaryDbContext Db, DeploymentEntity Deployment)> Setup()
    {
        var db = InMemoryPrimaryDb.Create();
        var deployment = NewDeployment();
        db.Deployments.Add(deployment);
        await db.SaveChangesAsync();
        return (db, deployment);
    }

    [Fact]
    public async Task TestApplyCheck_FirstAnswerIsUpWithoutEvent()
    {
        var (db, deployment) = await Setup();

        await Apply(db, deployment, Answer("1.2.3"));

        Assert.Equal(DeploymentEntity.StatusUp, deployment.Status);
        Assert.Equal("1.2.3", deployment.LiveVersion);
        Assert.Equal(Now.AddHours(-1), deployment.LiveDeployedAt);
        Assert.Equal(Now, deployment.StatusSince);
        Assert.Empty(db.DeploymentEvents);
    }

    [Fact]
    public async Task TestApplyCheck_NewVersionIsAnEvent()
    {
        var (db, deployment) = await Setup();
        await Apply(db, deployment, Answer("1.2.3"));

        await Apply(db, deployment, Answer("1.2.3"), minutes: 1);
        await Apply(db, deployment, Answer("1.3.0"), minutes: 2);

        var change = Assert.Single(db.DeploymentEvents);
        Assert.Equal(DeploymentEventEntity.KindVersionChanged, change.Kind);
        Assert.Equal("1.3.0", change.Version);
        Assert.Equal("1.2.3", change.PreviousVersion);
        Assert.Equal(Now, deployment.StatusSince);
    }

    [Fact]
    public async Task TestApplyCheck_DownAfterSecondFailureThenUpAgain()
    {
        var (db, deployment) = await Setup();
        await Apply(db, deployment, Answer("1.2.3"));

        await Apply(db, deployment, ProbeResult.Fail("timeout"), minutes: 1);
        Assert.Equal(DeploymentEntity.StatusUp, deployment.Status);
        Assert.Equal("timeout", deployment.CheckError);

        await Apply(db, deployment, ProbeResult.Fail("timeout"), minutes: 2);
        Assert.Equal(DeploymentEntity.StatusDown, deployment.Status);
        Assert.Equal(Now.AddMinutes(2), deployment.StatusSince);
        Assert.Equal("1.2.3", deployment.LiveVersion);

        await Apply(db, deployment, ProbeResult.Fail("timeout"), minutes: 3);
        await Apply(db, deployment, Answer("1.2.3"), minutes: 4);

        Assert.Equal(DeploymentEntity.StatusUp, deployment.Status);
        Assert.Null(deployment.CheckError);
        Assert.Equal(0, deployment.FailureCount);
        Assert.Equal([DeploymentEventEntity.KindDown, DeploymentEventEntity.KindUp],
            db.DeploymentEvents.OrderBy(e => e.At).Select(e => e.Kind));
    }

    [Fact]
    public async Task TestApplyCheck_AnotherApplicationIsAMismatch()
    {
        var (db, deployment) = await Setup();

        await Apply(db, deployment, Answer("9.9.9", application: "blog"));

        Assert.Equal(DeploymentEntity.StatusMismatch, deployment.Status);
        Assert.Null(deployment.LiveVersion);
        Assert.Contains("blog/prod", deployment.CheckError, StringComparison.Ordinal);
        Assert.Equal(DeploymentEventEntity.KindMismatch, Assert.Single(db.DeploymentEvents).Kind);
    }

    [Fact]
    public async Task TestDueForCheck_OnlyStaleWithAddress()
    {
        var db = InMemoryPrimaryDb.Create();
        var never = NewDeployment();
        var stale = NewDeployment();
        stale.CheckedAt = Now.AddMinutes(-2);
        var fresh = NewDeployment();
        fresh.CheckedAt = Now.AddSeconds(-10);
        var noAddress = NewDeployment();
        noAddress.Url = null;
        db.Deployments.AddRange(never, stale, fresh, noAddress);
        await db.SaveChangesAsync();

        var due = await MonitorOps.DueForCheck(Now, TimeSpan.FromMinutes(1), 10, new Read<DeploymentEntity>(db));

        Assert.Equal([never.Id, stale.Id], due.Select(d => d.Id).Order());
    }
}
