import { ConnectionInUse, fail, InvalidFormField, ok } from "@simpled/backend-client";
import { describe, expect, it, vi } from "vitest";
import { fakeClient, flush, makeConnection, makeOrg, signedInDeps } from "../../testing/fakes";
import { GitHubController } from "./github-controller";

function setup(parts: Parameters<typeof fakeClient>[0]["gitHubConnections"] = {}) {
  const client = fakeClient({
    gitHubConnections: {
      list: async () => ok([makeConnection({ id: 1, name: "Old bot", repositoryCount: 2 })]),
      ...parts,
    },
  });
  const controller = new GitHubController(signedInDeps(client, makeOrg("owner")), 1);
  return { controller, state: controller.state };
}

describe("GitHubController", () => {
  it("checks the form, then shows GitHub's refusal on the token field", async () => {
    const create = vi
      .fn()
      .mockResolvedValueOnce(fail(new InvalidFormField("token", "GitHub does not accept this token.")))
      .mockResolvedValueOnce(ok(makeConnection({ id: 2, name: "Deploy bot", gitHubLogin: "hubot" })));
    const { controller, state } = setup({ create });
    await flush();

    await controller.addConnection();
    expect(create).not.toHaveBeenCalled();
    expect(state.fieldErrors.value.token).toBeDefined();

    controller.edit({ name: " Deploy bot ", token: "ghp_wrong" });
    await controller.addConnection();
    expect(create).toHaveBeenCalledWith(expect.anything(), 1, { name: "Deploy bot", token: "ghp_wrong" });
    expect(state.fieldErrors.value.token).toBe("GitHub does not accept this token.");

    controller.edit({ token: "ghp_right" });
    await controller.addConnection();
    expect(state.connections.value.map((c) => c.name)).toEqual(["Deploy bot", "Old bot"]);
    expect(state.create.value.done).toBe("Connected as hubot.");
    expect(state.draft.value).toEqual({ name: "", token: "" });
  });

  it("renames without a new token, and keeps a connection still in use", async () => {
    const update = vi.fn(async () => ok(makeConnection({ id: 1, name: "Bot" })));
    const remove = vi.fn(async () => fail(new ConnectionInUse("2 linked repositories use this connection.")));
    const { controller, state } = setup({ update, delete: remove });
    await flush();

    controller.startEditing(1);
    controller.editConnection({ name: "Bot" });
    await controller.saveConnection();
    expect(update).toHaveBeenCalledWith(expect.anything(), 1, { name: "Bot", token: undefined });
    expect(state.editing.value).toBeNull();
    expect(state.connections.value[0].name).toBe("Bot");

    await controller.deleteConnection(1);
    expect(state.error.value).toBe("2 linked repositories use this connection.");
    expect(state.connections.value).toHaveLength(1);
  });
});
