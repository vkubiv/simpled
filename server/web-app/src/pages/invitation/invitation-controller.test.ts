import { fail, InvalidLink, type InvitationPreview, ok } from "@simpled/backend-client";
import { describe, expect, it, vi } from "vitest";
import { fakeClient, flush, makeDeps, makeMe, signedInDeps } from "../../testing/fakes";
import { InvitationController } from "./invitation-controller";

const PREVIEW: InvitationPreview = {
  organizationName: "Acme",
  role: "manager",
  email: "ann@example.com",
  invitedBy: "Bob",
  accountExists: false,
};

function previewing(preview: InvitationPreview) {
  return { invitations: { preview: async () => ok(preview) } };
}

describe("InvitationController", () => {
  it("signed out, a new email registers, then lands home signed in", async () => {
    const register = vi.fn(async () => ok(undefined));
    const client = fakeClient({
      invitations: { ...previewing(PREVIEW).invitations, register },
      me: { get: async () => ok(makeMe()) },
    });
    const deps = makeDeps(client);
    const routing = { onJoined: vi.fn() };
    const controller = new InvitationController(deps, routing, "tok");
    await flush();
    expect(controller.state.mode.value).toBe("register");

    controller.edit({ name: "Ann", password: "correct-horse-battery" });
    await controller.register();
    expect(register).toHaveBeenCalledWith(expect.anything(), "tok", "Ann", "correct-horse-battery");
    expect(deps.session.stage).toBe("signedIn");
    expect(routing.onJoined).toHaveBeenCalledWith(null);
  });

  it("signed out, an existing account signs in first", async () => {
    const controller = new InvitationController(
      makeDeps(fakeClient(previewing({ ...PREVIEW, accountExists: true }))),
      { onJoined: vi.fn() },
      "tok",
    );
    await flush();
    expect(controller.state.mode.value).toBe("signIn");
  });

  it("signed in as the invited email accepts; as someone else is told to switch", async () => {
    const accept = vi.fn(async () => ok({ id: 9, name: "Acme", slug: "acme", role: "manager" as const }));
    const client = fakeClient({
      invitations: { ...previewing(PREVIEW).invitations, accept },
      me: { get: async () => ok(makeMe()) },
    });
    const routing = { onJoined: vi.fn() };
    const controller = new InvitationController(signedInDeps(client), routing, "tok");
    await flush();
    expect(controller.state.mode.value).toBe("accept");
    await controller.accept();
    expect(routing.onJoined).toHaveBeenCalledWith(9);

    const other = new InvitationController(
      signedInDeps(fakeClient(previewing({ ...PREVIEW, email: "cid@example.com" }))),
      { onJoined: vi.fn() },
      "tok",
    );
    await flush();
    expect(other.state.mode.value).toBe("wrongAccount");
  });

  it("a used or expired link says so", async () => {
    const controller = new InvitationController(
      makeDeps(fakeClient({ invitations: { preview: async () => fail(new InvalidLink()) } })),
      { onJoined: vi.fn() },
      "tok",
    );
    await flush();
    expect(controller.state.load.value).toEqual({ kind: "error", message: "The link is invalid or has expired." });
  });
});
