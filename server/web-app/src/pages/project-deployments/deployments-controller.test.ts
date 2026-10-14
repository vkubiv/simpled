import { fail, InvalidFormField, ok } from "@simpled/backend-client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fakeClient,
  flush,
  makeConnection,
  makeDeployment,
  makeOrg,
  makeProject,
  makeRepository,
  signedInDeps,
} from "../../testing/fakes";
import { DeploymentsController, REFRESH_MS } from "./deployments-controller";

type Parts = Parameters<typeof fakeClient>[0];

function setup(canManage: boolean, repositories: Partial<NonNullable<Parts["repositories"]>> = {}) {
  const list = vi.fn(async () => ok([makeRepository()]));
  const client = fakeClient({
    projects: { get: async () => ok(makeProject({ canManage })) },
    gitHubConnections: { list: async () => ok([makeConnection({ id: 7 })]) },
    repositories: { list, ...repositories },
  });
  const controller = new DeploymentsController(signedInDeps(client, makeOrg(canManage ? "owner" : "member")), 1, 100);
  return { controller, state: controller.state, list };
}

describe("DeploymentsController", () => {
  beforeEach(() => vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] }));
  afterEach(() => vi.useRealTimers());

  it("shows the repositories, offers the only connection, and refreshes until disposed", async () => {
    const { controller, state, list } = setup(true);
    await flush();
    await flush();
    expect(state.repositories.value.map((r) => r.name)).toEqual(["shop"]);
    expect(state.draft.value.connectionId).toBe("7");

    list.mockResolvedValueOnce(ok([makeRepository({ environments: [] })]));
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    expect(state.repositories.value[0].environments).toEqual([]);

    controller.dispose();
    await vi.advanceTimersByTimeAsync(REFRESH_MS * 2);
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("a member sees deployments without the link form's connections", async () => {
    const { state } = setup(false);
    await flush();
    await flush();
    expect(state.canManage.value).toBe(false);
    expect(state.connections.value).toEqual([]);
  });

  it("links a repository; GitHub's refusal lands on the repository field", async () => {
    const link = vi
      .fn()
      .mockResolvedValueOnce(fail(new InvalidFormField("fullName", "No such repository.")))
      .mockResolvedValueOnce(ok(makeRepository({ id: 301, owner: "acme", name: "api" })));
    const { controller, state } = setup(true, { link });
    await flush();
    await flush();

    await controller.linkRepository();
    expect(link).not.toHaveBeenCalled();
    expect(state.fieldErrors.value.fullName).toBeDefined();

    controller.editLink({ fullName: "acme/nope" });
    await controller.linkRepository();
    expect(state.fieldErrors.value.fullName).toBe("No such repository.");

    controller.editLink({ fullName: " acme/api " });
    await controller.linkRepository();
    expect(link).toHaveBeenLastCalledWith(expect.anything(), 100, { connectionId: 7, fullName: "acme/api" });
    expect(state.repositories.value.map((r) => r.name)).toEqual(["api", "shop"]);
    expect(state.draft.value.fullName).toBe("");
  });

  it("opens one deployment's history at a time", async () => {
    const events = [
      { id: 1, at: new Date(), kind: "down" as const, version: "1.0.0", previousVersion: null, details: null },
    ];
    const deploymentEvents = vi.fn(async () => ok(events));
    const { controller, state } = setup(false, { deploymentEvents });
    await flush();

    await controller.toggleEvents(makeDeployment().id);
    expect(state.openEvents.value?.events).toEqual(events);
    await controller.toggleEvents(makeDeployment().id);
    expect(state.openEvents.value).toBeNull();
  });
});
