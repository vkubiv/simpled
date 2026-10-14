import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { createProject, loadProjects } from "../../core/ops/project-ops";
import { loadTeams } from "../../core/ops/team-ops";
import { fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { hasErrors, requiredError, slugError } from "../../core/primitives/forms";
import { busyForm, failedForm, failedLoad, idleForm, loaded, loading } from "../../core/primitives/load-status";
import { type ProjectDraft, ProjectsActions, ProjectsState } from "./projects-model";

export type ProjectsDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

export interface ProjectsRouting {
  onCreated(projectId: number): void;
}

export class ProjectsController extends SubtreeController implements ProjectsActions {
  readonly state = new ProjectsState();

  constructor(
    readonly deps: ProjectsDeps,
    readonly routing: ProjectsRouting,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(ProjectsState, this.state);
    this.subtree.put(ProjectsActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const o = { ...this.deps, orgId: this.orgId };
    const [projects, teams] = await Promise.all([loadProjects(o), loadTeams(o)]);
    if (this.isDisposed) return;
    if (!projects.ok || !teams.ok) {
      this.state.load.value = failedLoad([...(projects.ok ? [] : projects.errors), ...(teams.ok ? [] : teams.errors)]);
      return;
    }
    this.state.projects.value = projects.value;
    // A team the caller manages is one they may link: every team for owners and admins, a manager's own.
    this.state.teamOptions.value = teams.value.filter((t) => t.canManage);
    this.state.load.value = loaded;
  };

  edit = (patch: Partial<Omit<ProjectDraft, "teamIds">>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    this.state.fieldErrors.update((e) => ({
      ...e,
      ...Object.fromEntries(Object.keys(patch).map((k) => [k, undefined])),
    }));
  };

  toggleTeam = (teamId: number) => {
    this.state.draft.update((d) => ({
      ...d,
      teamIds: d.teamIds.includes(teamId) ? d.teamIds.filter((id) => id !== teamId) : [...d.teamIds, teamId],
    }));
    this.state.fieldErrors.update((e) => ({ ...e, teamIds: undefined }));
  };

  createProject = async () => {
    if (this.state.create.value.busy) return;
    const draft = this.state.draft.value;
    const manager = this.deps.orgRepository.org?.role === "manager";
    const errors = {
      name: requiredError(draft.name, "a project name"),
      slug: slugError(draft.slug.trim()),
      teamIds: manager && draft.teamIds.length === 0 ? "Pick at least one of your teams." : undefined,
    };
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.create.value = busyForm;
    const r = await createProject({
      ...this.deps,
      orgId: this.orgId,
      project: { ...draft, teamIds: [...draft.teamIds] },
    });
    if (this.isDisposed) return;
    if (r.ok) {
      this.state.create.value = idleForm;
      this.routing.onCreated(r.value.id);
      return;
    }
    this.state.fieldErrors.value = fieldErrors(r.errors);
    const rest = formLevelErrors(r.errors);
    this.state.create.value = rest.length > 0 ? failedForm(rest) : idleForm;
  };
}
