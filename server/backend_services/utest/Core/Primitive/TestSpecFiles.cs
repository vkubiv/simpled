using core.Primitive;
using utest.TestSupport;

namespace utest.Core.Primitive;

public class TestSpecFiles
{
    [Fact]
    public void TestReadEnvSpec_PrimaryHostAndExtends()
    {
        var spec = SpecFiles.ReadEnvSpec(MonitorSupport.EnvSpec);

        Assert.Empty(spec.Problems);
        var prod = spec.Deployments.Single(d => d.Name == "prod");
        Assert.Equal("shop", prod.ApplicationName);
        Assert.Equal("^1.2.0", prod.VersionConstraint);
        Assert.Equal("https://shop.example.com", prod.Url);
        var canary = spec.Deployments.Single(d => d.Name == "canary");
        Assert.Equal("shop", canary.ApplicationName);
        Assert.Equal("https://shop.example.com", canary.Url);
    }

    [Fact]
    public void TestReadEnvSpec_DockerAsksTheController()
    {
        var spec = SpecFiles.ReadEnvSpec(MonitorSupport.EnvSpec);

        var canary = spec.Deployments.Single(d => d.Name == "canary");
        Assert.Equal("https://ops.example.com/simpled/version/canary", canary.VersionUrl);
        Assert.Equal("https://ops.example.com/simpled/health/canary", canary.HealthUrl);

        var workers = SpecFiles.ReadEnvSpec("""
            type: docker
            gateway:
              tls: { upstream: true }
              hosts:
                ops: [ops.example.com]
            controller: { host: ops }
            deployments:
              jobs:
                application: { name: worker }
            """).Deployments.Single();
        Assert.Equal("https://ops.example.com", workers.Url);
        Assert.Equal("https://ops.example.com/simpled/version/jobs", workers.VersionUrl);

        var templated = SpecFiles.ReadEnvSpec("""
            type: docker
            gateway:
              hosts:
                web: shop.example.com
                ops: ops.$port(gateway)
            controller: { host: ops }
            deployments:
              prod:
                primary_host: web
                application: { name: shop }
            """).Deployments.Single();
        Assert.Equal("https://shop.example.com", templated.Url);
        Assert.Null(templated.VersionUrl);
        Assert.Null(templated.HealthUrl);
    }

    [Theory]
    [InlineData("type: k8s")]
    [InlineData("type: docker")]
    [InlineData("type: k8s\ncontroller: { host: web }")]
    public void TestReadEnvSpec_WellKnownWithoutAController(string head)
    {
        var deployment = SpecFiles.ReadEnvSpec(head + """

            gateway:
              hosts:
                web: shop.example.com
            deployments:
              prod:
                application: { name: shop }
            """).Deployments.Single();

        Assert.Equal("https://shop.example.com/.well-known/simpled/version", deployment.VersionUrl);
        Assert.Null(deployment.HealthUrl);
    }

    [Fact]
    public void TestReadEnvSpec_TlsDisabledSingleHostAndTemplates()
    {
        var spec = SpecFiles.ReadEnvSpec("""
            gateway:
              tls:
                disable: true
              hosts:
                app: localhost:$port(gateway)
            deployments:
              local:
                application:
                  name: shop
              other:
                primary_host: missing
                application:
                  name: shop
            """);

        Assert.Null(spec.Deployments.Single(d => d.Name == "local").Url);
        Assert.Null(spec.Deployments.Single(d => d.Name == "other").Url);
        Assert.Null(spec.Deployments.Single(d => d.Name == "other").VersionUrl);

        var plain = SpecFiles.ReadEnvSpec("""
            gateway:
              tls: { disable: true }
              hosts:
                app: [shop.test]
            deployments:
              dev:
                application: { name: shop }
            """);
        Assert.Equal("http://shop.test", plain.Deployments.Single().Url);
    }

    [Fact]
    public void TestReadEnvSpec_DeploymentWithoutApplicationIsAProblem()
    {
        var spec = SpecFiles.ReadEnvSpec("""
            deployments:
              broken:
                primary_host: web
              fine:
                application: { name: shop }
            """);

        Assert.Equal(["fine"], spec.Deployments.Select(d => d.Name));
        Assert.Contains(spec.Problems, p => p.Contains("broken", StringComparison.Ordinal));
    }

    [Theory]
    [InlineData("not: [valid")]
    [InlineData("- a list")]
    [InlineData("gateway: {}")]
    public void TestReadEnvSpec_Unreadable(string yaml)
    {
        Assert.Throws<SpecFormatException>(() => SpecFiles.ReadEnvSpec(yaml));
    }

    [Fact]
    public void TestReadAppSpec()
    {
        var spec = SpecFiles.ReadAppSpec(MonitorSupport.AppSpec);

        Assert.Equal("shop", spec.Name);
        Assert.Equal("1.2.3", spec.Version);
        Assert.Throws<SpecFormatException>(() => SpecFiles.ReadAppSpec("version: 1.0.0"));
    }

    [Theory]
    [InlineData("deployment/prod/envspec.yaml", "prod")]
    [InlineData("envs/dev/envspec.yml", "dev")]
    [InlineData("envspec.yaml", "shop")]
    public void TestEnvironmentName(string path, string name)
    {
        Assert.True(SpecFiles.IsEnvSpec(path));
        Assert.Equal(name, SpecFiles.EnvironmentName(path, "shop"));
    }

    [Fact]
    public void TestIsSpec_OnlyExactFileNames()
    {
        Assert.False(SpecFiles.IsEnvSpec("deployment/prod/envspec.yaml.bak"));
        Assert.False(SpecFiles.IsEnvSpec("localenv.yaml"));
        Assert.True(SpecFiles.IsAppSpec("hobby-pro/appspec.yaml"));
    }
}
