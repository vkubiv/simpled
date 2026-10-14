import { loadOrg } from "simpled-web-app/src/core/ops/org-ops";
import { InvitationController } from "simpled-web-app/src/pages/invitation/invitation-controller";
import { MembersController } from "simpled-web-app/src/pages/members/members-controller";
import { NewOrgController } from "simpled-web-app/src/pages/new-org/new-org-controller";
import { ProjectController } from "simpled-web-app/src/pages/project/project-controller";
import { ProjectsController } from "simpled-web-app/src/pages/projects/projects-controller";
import { TeamsController } from "simpled-web-app/src/pages/teams/teams-controller";
import { afterEach, describe, expect, test, vi } from "vitest";
import { expectSuccess } from "../utils/expect";
import { lastToken } from "../utils/mailbox";
import { PASSWORD, Sugar } from "../utils/sugar";
import { settled, signedInApp, WebApp, waitFor } from "../utils/web-app";

const apps: WebApp[] = [];
async function app(email: string): Promise<WebApp> {
  const a = await signedInApp(email);
  apps.push(a);
  return a;
}
afterEach(() => {
  for (const a of apps.splice(0)) a.dispose();
});

/** Pages render under the shell, which has loaded the organization; the tests do the same. */
async function showOrg(a: WebApp, orgId: number): Promise<void> {
  await loadOrg({ ...a.deps, orgId });
  expect(a.deps.orgRepository.org?.id).toBe(orgId);
}

describe("web app: organization", () => {
  test("an owner invites a new person, who registers through the link and is listed with the role", async () => {
    const sugar = new Sugar();
    const orgId = await sugar.org("ann");
    const owner = await app(sugar.email("ann"));
    await showOrg(owner, orgId);

    const members = owner.own(new MembersController(owner.deps, { onLeft: vi.fn() }, orgId));
    await settled(members.state.load, "members");
    members.editInvite({ email: sugar.email("bob"), role: "manager" });
    await members.sendInvite();
    expect(members.state.inviteStatus.value.done).toBe(`Invitation sent to ${sugar.email("bob")}.`);
    expect(members.state.invitations.value.map((i) => [i.email, i.role])).toEqual([[sugar.email("bob"), "manager"]]);

    const invited = new WebApp();
    apps.push(invited);
    const joined = { onJoined: vi.fn() };
    const page = invited.own(
      new InvitationController(invited.deps, joined, await lastToken(sugar.email("bob"), "invitation")),
    );
    await settled(page.state.load, "the invitation");
    expect(page.state.mode.value).toBe("register");
    page.edit({ name: "Bob", password: PASSWORD });
    await page.register();
    expect(joined.onJoined).toHaveBeenCalledWith(null);
    expect(invited.deps.meRepository.me?.organizations).toEqual([
      expect.objectContaining({ id: orgId, role: "manager" }),
    ]);

    await members.reload();
    const bob = members.state.members.value.find((m) => m.email === sugar.email("bob"));
    expect(bob).toMatchObject({ role: "manager", canChangeRole: true });
    expect(members.state.invitations.value).toEqual([]);

    await members.changeRole(bob?.accountId ?? "", "admin");
    expect(members.state.rowError.value).toBeNull();
    expect(members.state.members.value.find((m) => m.email === sugar.email("bob"))?.role).toBe("admin");
  });

  test("the last owner can't leave; a member leaves and goes home", async () => {
    const sugar = new Sugar();
    const orgId = await sugar.org("ann", { cid: "member" });
    const owner = await app(sugar.email("ann"));
    await showOrg(owner, orgId);
    const ownerPage = owner.own(new MembersController(owner.deps, { onLeft: vi.fn() }, orgId));
    await settled(ownerPage.state.load, "members");
    await ownerPage.remove(sugar.accountId("ann"));
    expect(ownerPage.state.rowError.value).toBe("An organization needs at least one owner.");

    const member = await app(sugar.email("cid"));
    await showOrg(member, orgId);
    const left = { onLeft: vi.fn() };
    const memberPage = member.own(new MembersController(member.deps, left, orgId));
    await settled(memberPage.state.load, "members");
    expect(memberPage.state.invitations.value).toEqual([]);
    await memberPage.remove(sugar.accountId("cid"));
    expect(left.onLeft).toHaveBeenCalled();
    expect(member.deps.meRepository.me?.organizations).toEqual([]);
  });

  test("a manager's team and project; linking another team needs managing both", async () => {
    const sugar = new Sugar();
    const orgId = await sugar.org("ann", { mia: "manager" });
    const frontend = expectSuccess(await sugar.client.teams.create(sugar.ctx("ann"), orgId, "Frontend"));
    const manager = await app(sugar.email("mia"));
    await showOrg(manager, orgId);

    const created = { onCreated: vi.fn() };
    const teams = manager.own(new TeamsController(manager.deps, created, orgId));
    await settled(teams.state.load, "teams");
    teams.setNewName("Backend");
    await teams.createTeam();
    const backendId = created.onCreated.mock.calls[0]?.[0] as number;
    expect(backendId).toBeGreaterThan(0);

    const projectCreated = { onCreated: vi.fn() };
    const projects = manager.own(new ProjectsController(manager.deps, projectCreated, orgId));
    await settled(projects.state.load, "projects");
    expect(projects.state.teamOptions.value.map((t) => t.name)).toEqual(["Backend"]);
    projects.edit({ name: "Web shop" });
    await projects.createProject();
    expect(projects.state.fieldErrors.value.teamIds).toBe("Pick at least one of your teams.");
    projects.toggleTeam(backendId);
    await projects.createProject();
    const projectId = projectCreated.onCreated.mock.calls[0]?.[0] as number;

    const asManager = manager.own(new ProjectController(manager.deps, { onDeleted: vi.fn() }, orgId, projectId));
    await settled(asManager.state.load, "the project");
    expect(asManager.state.project.value).toMatchObject({ slug: "web-shop", canManage: true });
    expect(asManager.state.linkable.value).toEqual([]);

    const owner = await app(sugar.email("ann"));
    await showOrg(owner, orgId);
    const asOwner = owner.own(new ProjectController(owner.deps, { onDeleted: vi.fn() }, orgId, projectId));
    await settled(asOwner.state.load, "the project");
    expect(asOwner.state.linkable.value.map((t) => t.id)).toEqual([frontend.id]);
    await asOwner.linkTeam(frontend.id);
    expect(asOwner.state.error.value).toBeNull();
    await waitFor(asOwner.state.project, (p) => p?.teams.length === 2, "the linked team");
  });

  test("another organization from the app is owned by its creator", async () => {
    const sugar = new Sugar();
    await sugar.register("ann");
    const a = await app(sugar.email("ann"));
    const created = { onCreated: vi.fn() };
    const page = a.own(new NewOrgController(a.deps, created));
    page.setName(sugar.orgName("Globex"));
    await page.submit();
    const orgId = created.onCreated.mock.calls[0]?.[0] as number;
    expect(a.deps.meRepository.me?.organizations).toContainEqual(
      expect.objectContaining({ id: orgId, role: "owner", name: sugar.orgName("Globex") }),
    );
  });
});
