import { fail, LastOwner, ok, RoleForbidden } from "@simpled/backend-client";
import { describe, expect, it, vi } from "vitest";
import { fakeClient, flush, makeMe, makeMember, makeOrg, signedInDeps } from "../../testing/fakes";
import { MembersController } from "./members-controller";

function setup(role: "owner" | "manager", parts: Parameters<typeof fakeClient>[0] = {}) {
  const members = vi.fn(async () =>
    ok([makeMember({ accountId: "acc-ann", name: "Ann", role, isMe: true }), makeMember()]),
  );
  const list = vi.fn(async () => ok([]));
  const client = fakeClient({
    ...parts,
    orgs: { members, ...parts.orgs },
    invitations: { list, ...parts.invitations },
    me: { get: async () => ok(makeMe()) },
  });
  const deps = signedInDeps(client, makeOrg(role));
  const routing = { onLeft: vi.fn() };
  const controller = new MembersController(deps, routing, 1);
  return { controller, state: controller.state, routing, members, list, deps };
}

describe("MembersController", () => {
  it("owners see invitations; managers see only the members", async () => {
    const owner = setup("owner");
    await flush();
    expect(owner.state.load.value.kind).toBe("loaded");
    expect(owner.list).toHaveBeenCalled();

    const manager = setup("manager");
    await flush();
    expect(manager.state.members.value).toHaveLength(2);
    expect(manager.list).not.toHaveBeenCalled();
  });

  it("an invitation is listed once sent, replacing an older one to the same email", async () => {
    const sent = {
      id: 2,
      email: "cid@example.com",
      role: "admin" as const,
      invitedBy: "Ann",
      createdAt: new Date(),
      expiresAt: new Date(),
      expired: false,
    };
    const invite = vi.fn(async () => ok(sent));
    const { controller, state } = setup("owner", { invitations: { invite } });
    await flush();
    state.invitations.value = [{ ...sent, id: 1, role: "member" }];

    controller.editInvite({ email: "cid@example.com", role: "admin" });
    await controller.sendInvite();

    expect(invite).toHaveBeenCalledWith(expect.anything(), 1, "cid@example.com", "admin");
    expect(state.invitations.value.map((i) => i.id)).toEqual([2]);
    expect(state.inviteStatus.value.done).toBe("Invitation sent to cid@example.com.");
    expect(state.invite.value.email).toBe("");
  });

  it("a refused role change shows the sentence", async () => {
    const setRole = vi.fn(async () =>
      fail(new RoleForbidden("Your role in this organization does not allow this.", "role")),
    );
    const { controller, state } = setup("owner", { orgs: { setRole } });
    await flush();
    await controller.changeRole("acc-bob", "admin");
    expect(state.rowError.value).toBe("Your role in this organization does not allow this.");
    expect(state.busyRow.value).toBeNull();
  });

  it("leaving goes home; the last owner is told why not", async () => {
    const removeMember = vi.fn().mockResolvedValueOnce(fail(new LastOwner())).mockResolvedValueOnce(ok(undefined));
    const { controller, state, routing, deps } = setup("owner", { orgs: { removeMember } });
    await flush();

    await controller.remove("acc-ann");
    expect(state.rowError.value).toBe("An organization needs at least one owner.");
    expect(routing.onLeft).not.toHaveBeenCalled();

    await controller.remove("acc-ann");
    expect(routing.onLeft).toHaveBeenCalled();
    expect(deps.orgRepository.org).toBeNull();
  });
});
