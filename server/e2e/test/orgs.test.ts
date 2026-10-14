import {
  InvalidFormField,
  LastOwner,
  MemberNotFound,
  OrganizationNotFound,
  RoleForbidden,
} from "@simpled/backend-client";
import { describe, expect, test } from "vitest";
import { expectError, expectSuccess } from "./utils/expect";
import { Sugar } from "./utils/sugar";

describe("organizations", () => {
  test("another organization is owned by its creator and listed in Me", async () => {
    const sugar = new Sugar();
    const { orgs, me } = sugar.client;
    await sugar.register("ann");

    const created = expectSuccess(await orgs.create(sugar.ctx("ann"), sugar.orgName("Globex")));
    expect(created.role).toBe("owner");
    expect(created.permissions).toMatchObject({ canManageOrganization: true, canCreateProject: true });
    expect(created.permissions.assignableRoles).toEqual(["owner", "admin", "manager", "member"]);

    const mine = expectSuccess(await me.get(sugar.ctx("ann"))).organizations.map((o) => o.id);
    expect(mine).toContain(created.id);

    expectError(await orgs.create(sugar.ctx("ann"), " "), InvalidFormField);
  });

  test("only the owner renames or deletes; outsiders see nothing", async () => {
    const sugar = new Sugar();
    const { orgs } = sugar.client;
    const orgId = await sugar.org("ann", { bob: "admin" });
    await sugar.register("eve");

    expectError(await orgs.rename(sugar.ctx("bob"), orgId, "Hijacked"), RoleForbidden);
    expectError(await orgs.delete(sugar.ctx("bob"), orgId), RoleForbidden);
    expectError(await orgs.get(sugar.ctx("eve"), orgId), OrganizationNotFound);
    expectError(await orgs.members(sugar.ctx("eve"), orgId), OrganizationNotFound);

    const renamed = expectSuccess(await orgs.rename(sugar.ctx("ann"), orgId, sugar.orgName("Acme Inc")));
    expect(renamed.name).toBe(sugar.orgName("Acme Inc"));

    expectSuccess(await orgs.delete(sugar.ctx("ann"), orgId));
    expectError(await orgs.get(sugar.ctx("ann"), orgId), OrganizationNotFound);
    expect(expectSuccess(await sugar.client.me.get(sugar.ctx("bob"))).organizations).toEqual([]);
  });

  test("members list flags what the caller may do", async () => {
    const sugar = new Sugar();
    const orgId = await sugar.org("ann", { bob: "admin", cid: "member" });

    const members = expectSuccess(await sugar.client.orgs.members(sugar.ctx("bob"), orgId));
    const byEmail = Object.fromEntries(members.map((m) => [m.email, m]));

    expect(byEmail[sugar.email("ann")]).toMatchObject({ role: "owner", canChangeRole: false, canRemove: false });
    expect(byEmail[sugar.email("bob")]).toMatchObject({ role: "admin", isMe: true, canRemove: true });
    expect(byEmail[sugar.email("cid")]).toMatchObject({ role: "member", canChangeRole: true, canRemove: true });
  });

  test("roles: admins manage non-owners, the last owner stays", async () => {
    const sugar = new Sugar();
    const { orgs } = sugar.client;
    const orgId = await sugar.org("ann", { bob: "admin", cid: "member" });

    expectSuccess(await orgs.setRole(sugar.ctx("bob"), orgId, sugar.accountId("cid"), "manager"));
    expectError(await orgs.setRole(sugar.ctx("bob"), orgId, sugar.accountId("cid"), "owner"), RoleForbidden);
    expectError(await orgs.setRole(sugar.ctx("bob"), orgId, sugar.accountId("ann"), "member"), RoleForbidden);
    expectError(await orgs.setRole(sugar.ctx("cid"), orgId, sugar.accountId("bob"), "member"), RoleForbidden);

    expectError(await orgs.setRole(sugar.ctx("ann"), orgId, sugar.accountId("ann"), "admin"), LastOwner);
    expectSuccess(await orgs.setRole(sugar.ctx("ann"), orgId, sugar.accountId("bob"), "owner"));
    expectSuccess(await orgs.setRole(sugar.ctx("ann"), orgId, sugar.accountId("ann"), "admin"));

    const roles = expectSuccess(await orgs.members(sugar.ctx("ann"), orgId)).map((m) => [m.email, m.role]);
    expect(Object.fromEntries(roles)).toEqual({
      [sugar.email("ann")]: "admin",
      [sugar.email("bob")]: "owner",
      [sugar.email("cid")]: "manager",
    });
  });

  test("removing and leaving", async () => {
    const sugar = new Sugar();
    const { orgs } = sugar.client;
    const orgId = await sugar.org("ann", { bob: "member", cid: "member", dan: "admin" });
    await sugar.register("eve");

    expectError(await orgs.removeMember(sugar.ctx("bob"), orgId, sugar.accountId("cid")), RoleForbidden);
    expectError(await orgs.removeMember(sugar.ctx("ann"), orgId, sugar.accountId("eve")), MemberNotFound);
    expectError(await orgs.removeMember(sugar.ctx("ann"), orgId, sugar.accountId("ann")), LastOwner);

    expectSuccess(await orgs.removeMember(sugar.ctx("bob"), orgId, sugar.accountId("bob")));
    expectSuccess(await orgs.removeMember(sugar.ctx("dan"), orgId, sugar.accountId("cid")));
    expectError(await orgs.get(sugar.ctx("cid"), orgId), OrganizationNotFound);

    const left = expectSuccess(await orgs.members(sugar.ctx("ann"), orgId)).map((m) => m.email);
    expect(left.sort()).toEqual([sugar.email("ann"), sugar.email("dan")].sort());
  });
});
