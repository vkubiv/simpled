import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { loadMembers } from "../../core/ops/org-ops";
import { loadProjects } from "../../core/ops/project-ops";
import { loadTeams } from "../../core/ops/team-ops";
import { failedLoad, loaded, loading } from "../../core/primitives/load-status";
import { OrgOverviewActions, OrgOverviewState } from "./org-overview-model";

export type OrgOverviewDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

/** The organization at a glance: counts, the caller's teams and the projects. */
export class OrgOverviewController extends SubtreeController implements OrgOverviewActions {
  readonly state = new OrgOverviewState();

  constructor(
    readonly deps: OrgOverviewDeps,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(OrgOverviewState, this.state);
    this.subtree.put(OrgOverviewActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const o = { ...this.deps, orgId: this.orgId };
    const [teams, projects, members] = await Promise.all([loadTeams(o), loadProjects(o), loadMembers(o)]);
    if (this.isDisposed) return;
    if (!teams.ok || !projects.ok || !members.ok) {
      const failed = [teams, projects, members].flatMap((r) => (r.ok ? [] : r.errors));
      this.state.load.value = failedLoad(failed);
      return;
    }
    this.state.teams.value = teams.value;
    this.state.projects.value = projects.value;
    this.state.memberCount.value = members.value.length;
    this.state.load.value = loaded;
  };
}
