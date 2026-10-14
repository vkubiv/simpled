import {
  ConnectionInUse,
  DeploymentNotFound,
  InvalidFormField,
  ProjectNotFound,
  type Repository,
  RepositoryNotFound,
  RoleForbidden,
} from "@simpled/backend-client";
import { describe, expect, test } from "vitest";
import { eventually } from "./utils/eventually";
import { expectError, expectSuccess } from "./utils/expect";
import { appSpec, envSpec, Fakes } from "./utils/fakes";
import { Sugar } from "./utils/sugar";

function deployment(repository: Repository, environment: string, name: string) {
  const found = repository.environments.find((e) => e.name === environment)?.deployments.find((d) => d.name === name);
  if (!found) throw new Error(`No deployment ${environment}/${name} in ${JSON.stringify(repository.environments)}`);
  return found;
}

describe("GitHub connections", () => {
  test("owners and admins add tokens GitHub accepts; members see them without the token", async () => {
    const sugar = new Sugar();
    const fakes = new Fakes();
    const { gitHubConnections } = sugar.client;
    const orgId = await sugar.org("ann", { ada: "admin", max: "member" });
    const token = fakes.unique("ghp");
    await fakes.token(token, "acme-bot");

    const created = expectSuccess(await gitHubConnections.create(sugar.ctx("ada"), orgId, { name: "Deploys", token }));
    expect([created.name, created.gitHubLogin, created.repositoryCount]).toEqual(["Deploys", "acme-bot", 0]);
    expect(JSON.stringify(created)).not.toContain(token);

    const refused = expectError(
      await gitHubConnections.create(sugar.ctx("ann"), orgId, { name: "Other", token: fakes.unique("wrong") }),
      InvalidFormField,
    );
    expect(refused.fieldName).toBe("token");
    expectError(await gitHubConnections.create(sugar.ctx("max"), orgId, { name: "Mine", token }), RoleForbidden);
    expectError(await gitHubConnections.create(sugar.ctx("ann"), orgId, { name: "Deploys", token }), InvalidFormField);

    const listed = expectSuccess(await gitHubConnections.list(sugar.ctx("max"), orgId));
    expect(listed.map((c) => c.name)).toEqual(["Deploys"]);
    expectError(await gitHubConnections.update(sugar.ctx("max"), created.id, { name: "Renamed" }), RoleForbidden);
    const renamed = expectSuccess(await gitHubConnections.update(sugar.ctx("ann"), created.id, { name: "Renamed" }));
    expect(renamed.gitHubLogin).toBe("acme-bot");
  });
});

describe("monitoring", () => {
  test("a linked repository's envspec becomes deployments the monitor follows", async () => {
    const sugar = new Sugar();
    const fakes = new Fakes();
    const { gitHubConnections, projects, repositories } = sugar.client;
    const orgId = await sugar.org("ann", { max: "member" });
    const ann = sugar.ctx("ann");

    const token = fakes.unique("ghp");
    const owner = fakes.unique("acme");
    const host = `${fakes.unique("shop")}.test`;
    await fakes.token(token);
    const files = {
      "deployment/prod/envspec.yaml": envSpec(host, { prod: { application: "shop", version: "^1.0.0" } }),
      "app/appspec.yaml": appSpec("shop", "1.1.0"),
    };
    await fakes.repository(owner, "shop", [token], files);
    await fakes.site(host, { application: "shop", deployment: "prod", version: "1.0.0" });

    const connection = expectSuccess(await gitHubConnections.create(ann, orgId, { name: "Bot", token }));
    const project = expectSuccess(await projects.create(ann, orgId, { name: "Shop" }));
    const linked = expectSuccess(
      await repositories.link(ann, project.id, { connectionId: connection.id, fullName: `${owner}/shop` }),
    );
    expect(linked.syncError).toBeNull();
    const prod = deployment(linked, "prod", "prod");
    expect([prod.applicationName, prod.versionConstraint, prod.repoVersion, prod.url]).toEqual([
      "shop",
      "^1.0.0",
      "1.1.0",
      `http://${host}`,
    ]);

    const read = async () =>
      deployment(expectSuccess(await repositories.get(sugar.ctx("max"), linked.id)), "prod", "prod");
    const up = await eventually(read, (d) => d.status === "up", "prod up");
    expect(up.liveVersion).toBe("1.0.0");
    expect(up.liveDeployedAt).not.toBeNull();

    await fakes.site(host, { application: "shop", deployment: "prod", version: "1.1.0" });
    await eventually(read, (d) => d.liveVersion === "1.1.0", "the new version");

    await fakes.siteDown(host);
    const down = await eventually(read, (d) => d.status === "down", "prod down");
    expect(down.checkError).toContain("502");

    const events = expectSuccess(await repositories.deploymentEvents(sugar.ctx("max"), prod.id));
    expect(events.map((e) => [e.kind, e.version, e.previousVersion])).toEqual([
      ["down", "1.1.0", null],
      ["version-changed", "1.1.0", "1.0.0"],
    ]);

    await fakes.repository(owner, "shop", [token], {
      ...files,
      "deployment/staging/envspec.yaml": envSpec(`staging-${host}`, { staging: { application: "shop" } }),
    });
    expectError(await repositories.sync(sugar.ctx("max"), linked.id), RoleForbidden);
    const synced = expectSuccess(await repositories.sync(ann, linked.id));
    expect(synced.environments.map((e) => [e.name, e.deployments.map((d) => d.name)])).toEqual([
      ["prod", ["prod"]],
      ["staging", ["staging"]],
    ]);
    expect(deployment(synced, "prod", "prod").status).toBe("down");

    expectError(await gitHubConnections.delete(ann, connection.id), ConnectionInUse);
    expectError(await repositories.unlink(sugar.ctx("max"), linked.id), RoleForbidden);
    expectSuccess(await repositories.unlink(ann, linked.id));
    expect(expectSuccess(await repositories.list(ann, project.id))).toEqual([]);
    expectError(await repositories.deploymentEvents(ann, prod.id), DeploymentNotFound);
    expectSuccess(await gitHubConnections.delete(ann, connection.id));
  });

  test("linking needs a repository the token reads, once, by someone who manages the project", async () => {
    const sugar = new Sugar();
    const fakes = new Fakes();
    const { gitHubConnections, projects, repositories } = sugar.client;
    const orgId = await sugar.org("ann", { max: "member" });
    await sugar.register("eve", "Evil Corp");
    const ann = sugar.ctx("ann");

    const token = fakes.unique("ghp");
    const owner = fakes.unique("acme");
    await fakes.token(token);
    await fakes.repository(owner, "shop", [token], { "README.md": "# shop" });
    await fakes.repository(owner, "secret", [fakes.unique("other")], { "README.md": "# secret" });
    const connection = expectSuccess(await gitHubConnections.create(ann, orgId, { name: "Bot", token }));
    const project = expectSuccess(await projects.create(ann, orgId, { name: "Shop" }));
    const link = (fullName: string) => ({ connectionId: connection.id, fullName });

    const hidden = expectError(await repositories.link(ann, project.id, link(`${owner}/secret`)), InvalidFormField);
    expect(hidden.fieldName).toBe("fullName");
    expectError(await repositories.link(ann, project.id, link("not a repository")), InvalidFormField);
    expectError(await repositories.link(sugar.ctx("max"), project.id, link(`${owner}/shop`)), RoleForbidden);
    expectError(await repositories.link(sugar.ctx("eve"), project.id, link(`${owner}/shop`)), ProjectNotFound);

    const linked = expectSuccess(await repositories.link(ann, project.id, link(`https://github.com/${owner}/shop`)));
    expect(linked.environments).toEqual([]);
    const again = expectError(await repositories.link(ann, project.id, link(`${owner}/shop`)), InvalidFormField);
    expect(again.fieldName).toBe("fullName");

    expectError(await repositories.get(sugar.ctx("eve"), linked.id), RepositoryNotFound);
    const listed = expectSuccess(await repositories.list(sugar.ctx("max"), project.id));
    expect(listed.map((r) => [r.owner, r.name, r.canManage])).toEqual([[owner, "shop", false]]);
  });
});
