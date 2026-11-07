import { loadOrg } from "simpled-web-app/src/core/ops/org-ops";
import { GitHubController } from "simpled-web-app/src/pages/github/github-controller";
import { DeploymentsController } from "simpled-web-app/src/pages/project-deployments/deployments-controller";
import { afterEach, describe, expect, test } from "vitest";
import { eventually } from "../utils/eventually";
import { expectSuccess } from "../utils/expect";
import { appSpec, envSpec, Fakes } from "../utils/fakes";
import { Sugar } from "../utils/sugar";
import { settled, signedInApp, type WebApp } from "../utils/web-app";

const apps: WebApp[] = [];
afterEach(() => {
  for (const a of apps.splice(0)) a.dispose();
});

describe("web app: monitoring", () => {
  test("an owner connects GitHub, links a repository and watches its deployment come up", async () => {
    const sugar = new Sugar();
    const fakes = new Fakes();
    const orgId = await sugar.org("ann");
    const project = expectSuccess(await sugar.client.projects.create(sugar.ctx("ann"), orgId, { name: "Shop" }));
    const token = fakes.unique("ghp");
    const owner = fakes.unique("acme");
    const host = `${fakes.unique("web")}.test`;
    await fakes.token(token, "acme-bot");
    await fakes.repository(owner, "shop", [token], {
      "deploy/prod/envspec.yaml": envSpec(host, { prod: { application: "shop" } }),
      "appspec.yaml": appSpec("shop", "2.0.0"),
    });
    await fakes.controller(host, { application: "shop", deployment: "prod", version: "1.9.0" });

    const app = await signedInApp(sugar.email("ann"));
    apps.push(app);
    await loadOrg({ ...app.deps, orgId });

    const github = app.own(new GitHubController(app.deps, orgId));
    await settled(github.state.load, "connections");
    github.edit({ name: "Deploy bot", token });
    await github.addConnection();
    expect(github.state.create.value.done).toBe("Connected as acme-bot.");

    const deployments = app.own(new DeploymentsController(app.deps, orgId, project.id));
    await settled(deployments.state.load, "deployments");
    await eventually(
      async () => deployments.state.connections.value,
      (c) => c.length === 1,
      "the connection offered",
    );
    expect(deployments.state.draft.value.connectionId).toBe(String(github.state.connections.value[0].id));
    deployments.editLink({ fullName: `${owner}/shop` });
    await deployments.linkRepository();
    expect(deployments.state.link.value.error).toBeNull();

    const prod = () => deployments.state.repositories.value[0].environments[0].deployments[0];
    expect([prod().name, prod().repoVersion]).toEqual(["prod", "2.0.0"]);
    await eventually(
      async () => {
        await deployments.refresh();
        return prod();
      },
      (d) => d.status === "up",
      "prod up in the page",
    );
    expect(prod().liveVersion).toBe("1.9.0");
  });
});
