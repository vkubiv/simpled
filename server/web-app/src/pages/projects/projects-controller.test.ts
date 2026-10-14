import { fail, InvalidFormField, ok } from "@simpled/backend-client";
import { describe, expect, it, type Mock, vi } from "vitest";
import { fakeClient, flush, makeOrg, makeProject, makeTeamSummary, signedInDeps } from "../../testing/fakes";
import { ProjectsController } from "./projects-controller";

function setup(role: "owner" | "manager", create: Mock = vi.fn(async () => ok(makeProject()))) {
  const teams = [
    makeTeamSummary({ id: 10, name: "Backend", isMember: true, canManage: true }),
    makeTeamSummary({ id: 11, name: "Frontend", canManage: role === "owner" }),
  ];
  const client = fakeClient({
    projects: { list: async () => ok([]), create },
    teams: { list: async () => ok(teams) },
  });
  const routing = { onCreated: vi.fn() };
  const controller = new ProjectsController(signedInDeps(client, makeOrg(role)), routing, 1);
  return { controller, state: controller.state, routing, create };
}

describe("ProjectsController", () => {
  it("offers every team to an owner, and a manager only their own", async () => {
    const owner = setup("owner");
    const manager = setup("manager");
    await flush();
    expect(owner.state.teamOptions.value.map((t) => t.name)).toEqual(["Backend", "Frontend"]);
    expect(manager.state.teamOptions.value.map((t) => t.name)).toEqual(["Backend"]);
  });

  it("a manager must pick one of their teams", async () => {
    const { controller, state, create } = setup("manager");
    await flush();
    controller.edit({ name: "Api" });
    await controller.createProject();
    expect(create).not.toHaveBeenCalled();
    expect(state.fieldErrors.value.teamIds).toBe("Pick at least one of your teams.");

    controller.toggleTeam(10);
    await controller.createProject();
    expect(create).toHaveBeenCalledWith(expect.anything(), 1, { name: "Api", slug: undefined, teamIds: [10] });
  });

  it("creates and opens the project; a taken slug lands on its field", async () => {
    const create = vi
      .fn()
      .mockResolvedValueOnce(fail(new InvalidFormField("slug", "A project with this slug already exists.")))
      .mockResolvedValueOnce(ok(makeProject({ id: 101 })));
    const { controller, state, routing } = setup("owner", create);
    await flush();
    controller.edit({ name: "Web shop", slug: "web" });

    await controller.createProject();
    expect(state.fieldErrors.value.slug).toBe("A project with this slug already exists.");
    expect(routing.onCreated).not.toHaveBeenCalled();

    controller.edit({ slug: "web-2" });
    await controller.createProject();
    expect(routing.onCreated).toHaveBeenCalledWith(101);
  });
});
