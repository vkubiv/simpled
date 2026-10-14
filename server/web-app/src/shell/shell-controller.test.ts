import { fail, OrganizationNotFound, ok } from "@simpled/backend-client";
import { describe, expect, it, vi } from "vitest";
import { fakeClient, flush, makeDeps, makeMe, makeOrg } from "../testing/fakes";
import { ShellController } from "./shell-controller";

describe("ShellController", () => {
  it("loads the organization in the URL and lists the account's others", async () => {
    const get = vi.fn(async () => ok(makeOrg("admin", { id: 2, name: "Globex" })));
    const deps = makeDeps(fakeClient({ orgs: { get } }), "signedIn");
    deps.meRepository.set({ kind: "loaded", me: makeMe([{ id: 1, name: "Acme", slug: "acme", role: "owner" }]) });
    deps.orgRepository.set({ kind: "loaded", org: makeOrg() });

    const controller = new ShellController(deps, 2);
    await flush();

    expect(get).toHaveBeenCalledWith(expect.anything(), 2);
    expect(controller.state.view.value).toBe("ready");
    expect(controller.state.org.value?.name).toBe("Globex");
    expect(controller.state.organizations.value.map((o) => o.name)).toEqual(["Acme"]);
    controller.dispose();
  });

  it("an organization the caller is not in is not found", async () => {
    const deps = makeDeps(fakeClient({ orgs: { get: async () => fail(new OrganizationNotFound(3)) } }), "signedIn");
    const controller = new ShellController(deps, 3);
    await flush();
    expect(controller.state.view.value).toBe("notFound");
    controller.dispose();
  });
});
