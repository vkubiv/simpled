import { ApiNotAuthorized, fail, ok } from "@simpled/backend-client";
import { describe, expect, it, vi } from "vitest";
import { AppController } from "./app-controller";
import { signIn } from "./core/ops/auth-ops";
import { fakeClient, flush, makeDeps, makeMe, makeOrg } from "./testing/fakes";

describe("AppController", () => {
  it("restores a signed-in session from /Me and sends home to the first organization", async () => {
    const get = vi.fn(async () => ok(makeMe([{ id: 7, name: "Acme", slug: "acme", role: "owner" }])));
    const deps = makeDeps(fakeClient({ me: { get } }), "restoring");
    const controller = new AppController(deps);
    await flush();

    expect(get).toHaveBeenCalledTimes(1);
    expect(controller.state.gate.value).toBe("signedIn");
    expect(controller.state.home.value).toBe("/orgs/7");
    controller.dispose();
  });

  it("a 401 from /Me is signed out; an account without organizations goes to create one", async () => {
    const deps = makeDeps(fakeClient({ me: { get: async () => fail(new ApiNotAuthorized("/Me")) } }), "restoring");
    const controller = new AppController(deps);
    await flush();
    expect(controller.state.gate.value).toBe("signedOut");
    expect(controller.state.home.value).toBe("/orgs/new");
    controller.dispose();
  });

  it("signing in after a signed-out start keeps the account it loads", async () => {
    const get = vi
      .fn()
      .mockResolvedValueOnce(fail(new ApiNotAuthorized("/Me")))
      .mockResolvedValue(ok(makeMe([{ id: 7, name: "Acme", slug: "acme", role: "owner" }])));
    const deps = makeDeps(fakeClient({ me: { get }, auth: { login: async () => ok(undefined) } }), "restoring");
    const controller = new AppController(deps);
    await flush();
    expect(controller.state.gate.value).toBe("signedOut");

    await signIn({ ...deps, email: "ann@example.com", password: "correct-horse-battery" });
    await flush();

    expect(controller.state.gate.value).toBe("signedIn");
    expect(deps.meRepository.me?.organizations).toHaveLength(1);
    expect(controller.state.home.value).toBe("/orgs/7");
    controller.dispose();
  });

  it("main-svc not answering is a retry, not the sign-in form", async () => {
    const get = vi.fn().mockRejectedValueOnce(new Error("fetch failed")).mockResolvedValueOnce(ok(makeMe()));
    const controller = new AppController(makeDeps(fakeClient({ me: { get } }), "restoring"));
    await flush();
    expect(controller.state.gate.value).toBe("failed");
    expect(controller.state.failure.value).toBe("fetch failed");

    controller.retry();
    await flush();
    expect(controller.state.gate.value).toBe("signedIn");
    controller.dispose();
  });

  it("signing out forgets the account and the organization shown", async () => {
    const deps = makeDeps(fakeClient({ auth: { logout: async () => ok(undefined) } }), "signedIn");
    deps.meRepository.set({ kind: "loaded", me: makeMe() });
    deps.orgRepository.set({ kind: "loaded", org: makeOrg() });
    const controller = new AppController(deps);

    await controller.signOut();
    await flush();

    expect(controller.state.gate.value).toBe("signedOut");
    expect(deps.meRepository.me).toBeNull();
    expect(deps.orgRepository.org).toBeNull();
    controller.dispose();
  });
});
