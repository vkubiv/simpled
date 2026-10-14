import {
  AlreadyMember,
  InvalidFormField,
  MemberNotFound,
  OrganizationNotFound,
  ProjectNotFound,
  RoleForbidden,
  TeamNotFound,
} from "@simpled/backend-client";
import { describe, expect, test } from "vitest";
import { expectError, expectSuccess } from "./utils/expect";
import { Sugar } from "./utils/sugar";

describe("teams", () => {
  test("a manager creates a team, joins it, and manages only their own teams", async () => {
    const sugar = new Sugar();
    const { teams } = sugar.client;
    const orgId = await sugar.org("ann", { mia: "manager", cid: "member" });

    const backend = expectSuccess(await teams.create(sugar.ctx("mia"), orgId, "Backend"));
    expect(backend.members.map((m) => m.email)).toEqual([sugar.email("mia")]);
    expect(backend.canManage).toBe(true);
    const frontend = expectSuccess(await teams.create(sugar.ctx("ann"), orgId, "Frontend"));
    expect(frontend.members).toEqual([]);

    expectError(await teams.create(sugar.ctx("cid"), orgId, "Ops"), RoleForbidden);
    expectError(await teams.create(sugar.ctx("ann"), orgId, "Backend"), InvalidFormField);

    expectSuccess(await teams.addMember(sugar.ctx("mia"), backend.id, sugar.accountId("cid")));
    expectError(await teams.addMember(sugar.ctx("mia"), backend.id, sugar.accountId("cid")), AlreadyMember);
    expectError(await teams.addMember(sugar.ctx("mia"), frontend.id, sugar.accountId("cid")), RoleForbidden);
    expectError(await teams.rename(sugar.ctx("mia"), frontend.id, "Web"), RoleForbidden);
    expectError(await teams.delete(sugar.ctx("cid"), backend.id), RoleForbidden);

    const renamed = expectSuccess(await teams.rename(sugar.ctx("mia"), backend.id, "Platform"));
    expect(renamed.members.map((m) => [m.email, m.role])).toEqual([
      [sugar.email("cid"), "member"],
      [sugar.email("mia"), "manager"],
    ]);

    const listed = expectSuccess(await teams.list(sugar.ctx("cid"), orgId));
    expect(listed.map((t) => [t.name, t.memberCount, t.isMember, t.canManage])).toEqual([
      ["Frontend", 0, false, false],
      ["Platform", 2, true, false],
    ]);

    expectSuccess(await teams.removeMember(sugar.ctx("mia"), backend.id, sugar.accountId("cid")));
    expectError(await teams.removeMember(sugar.ctx("mia"), backend.id, sugar.accountId("cid")), MemberNotFound);
    expectSuccess(await teams.delete(sugar.ctx("mia"), backend.id));
    expectError(await teams.get(sugar.ctx("ann"), backend.id), TeamNotFound);
  });

  test("teams of other organizations are not found, and only members join", async () => {
    const sugar = new Sugar();
    const { teams } = sugar.client;
    const orgId = await sugar.org("ann");
    await sugar.register("eve", "Evil Corp");
    const team = expectSuccess(await teams.create(sugar.ctx("ann"), orgId, "Backend"));

    expectError(await teams.get(sugar.ctx("eve"), team.id), TeamNotFound);
    expectError(await teams.list(sugar.ctx("eve"), orgId), OrganizationNotFound);
    expectError(await teams.addMember(sugar.ctx("ann"), team.id, sugar.accountId("eve")), MemberNotFound);
  });

  test("removing someone from the organization takes them out of its teams", async () => {
    const sugar = new Sugar();
    const orgId = await sugar.org("ann", { bob: "member" });
    const team = expectSuccess(await sugar.client.teams.create(sugar.ctx("ann"), orgId, "Backend"));
    expectSuccess(await sugar.client.teams.addMember(sugar.ctx("ann"), team.id, sugar.accountId("bob")));

    expectSuccess(await sugar.client.orgs.removeMember(sugar.ctx("ann"), orgId, sugar.accountId("bob")));

    expect(expectSuccess(await sugar.client.teams.get(sugar.ctx("ann"), team.id)).members).toEqual([]);
  });
});

describe("projects", () => {
  test("slugs come from the name unless given, unique within the organization", async () => {
    const sugar = new Sugar();
    const { projects } = sugar.client;
    const orgId = await sugar.org("ann");

    const first = expectSuccess(await projects.create(sugar.ctx("ann"), orgId, { name: "Web Shop" }));
    const second = expectSuccess(await projects.create(sugar.ctx("ann"), orgId, { name: "Web Shop" }));
    const given = expectSuccess(await projects.create(sugar.ctx("ann"), orgId, { name: "Api", slug: "api-v2" }));
    expect([first.slug, second.slug, given.slug]).toEqual(["web-shop", "web-shop-2", "api-v2"]);

    const taken = expectError(
      await projects.create(sugar.ctx("ann"), orgId, { name: "X", slug: "api-v2" }),
      InvalidFormField,
    );
    expect(taken.fieldName).toBe("slug");
    expectError(
      await projects.update(sugar.ctx("ann"), first.id, { name: "Web", slug: "Not A Slug" }),
      InvalidFormField,
    );

    const updated = expectSuccess(await projects.update(sugar.ctx("ann"), first.id, { name: "Web", slug: "web" }));
    expect([updated.name, updated.slug]).toEqual(["Web", "web"]);
  });

  test("a manager's projects link their teams; they manage only those", async () => {
    const sugar = new Sugar();
    const { teams, projects } = sugar.client;
    const orgId = await sugar.org("ann", { mia: "manager", cid: "member" });
    const own = expectSuccess(await teams.create(sugar.ctx("mia"), orgId, "Backend"));
    const other = expectSuccess(await teams.create(sugar.ctx("ann"), orgId, "Frontend"));

    const noTeam = expectError(await projects.create(sugar.ctx("mia"), orgId, { name: "Api" }), RoleForbidden);
    expect(noTeam.fieldName).toBe("teamIds");
    expectError(
      await projects.create(sugar.ctx("mia"), orgId, { name: "Api", teamIds: [own.id, other.id] }),
      RoleForbidden,
    );
    expectError(await projects.create(sugar.ctx("cid"), orgId, { name: "Api", teamIds: [own.id] }), RoleForbidden);
    const unknown = expectError(
      await projects.create(sugar.ctx("ann"), orgId, { name: "Api", teamIds: [own.id, 999_999_999] }),
      InvalidFormField,
    );
    expect(unknown.fieldName).toBe("teamIds");

    const api = expectSuccess(await projects.create(sugar.ctx("mia"), orgId, { name: "Api", teamIds: [own.id] }));
    expect(api).toMatchObject({ isMember: true, canManage: true, teams: [{ id: own.id, name: "Backend" }] });
    const web = expectSuccess(await projects.create(sugar.ctx("ann"), orgId, { name: "Web", teamIds: [other.id] }));

    expectError(await projects.update(sugar.ctx("mia"), web.id, { name: "Web 2", slug: "web-2" }), RoleForbidden);
    expectError(await projects.delete(sugar.ctx("mia"), web.id), RoleForbidden);
    // Linking needs both the project and the team.
    expectError(await projects.linkTeam(sugar.ctx("mia"), api.id, other.id), RoleForbidden);
    expectError(await projects.linkTeam(sugar.ctx("mia"), web.id, own.id), RoleForbidden);

    const listed = expectSuccess(await projects.list(sugar.ctx("cid"), orgId));
    expect(listed.map((p) => [p.name, p.isMember, p.canManage])).toEqual([
      ["Api", false, false],
      ["Web", false, false],
    ]);

    expectSuccess(await projects.delete(sugar.ctx("mia"), api.id));
    expectError(await projects.get(sugar.ctx("ann"), api.id), ProjectNotFound);
  });

  test("linking and unlinking teams; a deleted team leaves its projects", async () => {
    const sugar = new Sugar();
    const { teams, projects } = sugar.client;
    const orgId = await sugar.org("ann");
    const backend = expectSuccess(await teams.create(sugar.ctx("ann"), orgId, "Backend"));
    const frontend = expectSuccess(await teams.create(sugar.ctx("ann"), orgId, "Frontend"));
    const project = expectSuccess(
      await projects.create(sugar.ctx("ann"), orgId, { name: "Shop", teamIds: [backend.id] }),
    );

    expectSuccess(await projects.linkTeam(sugar.ctx("ann"), project.id, frontend.id));
    expectError(await projects.linkTeam(sugar.ctx("ann"), project.id, frontend.id), AlreadyMember);
    expect(expectSuccess(await projects.get(sugar.ctx("ann"), project.id)).teams.map((t) => t.name)).toEqual([
      "Backend",
      "Frontend",
    ]);
    expect(expectSuccess(await teams.get(sugar.ctx("ann"), frontend.id)).projects.map((p) => p.slug)).toEqual(["shop"]);

    expectSuccess(await projects.unlinkTeam(sugar.ctx("ann"), project.id, frontend.id));
    expectError(await projects.unlinkTeam(sugar.ctx("ann"), project.id, frontend.id), TeamNotFound);

    expectSuccess(await teams.delete(sugar.ctx("ann"), backend.id));
    expect(expectSuccess(await projects.get(sugar.ctx("ann"), project.id)).teams).toEqual([]);
  });
});
