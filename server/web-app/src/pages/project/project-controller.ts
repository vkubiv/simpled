import type { Project, TeamSummary } from "@simpled/backend-client";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { deleteProject, linkTeam, loadProject, unlinkTeam, updateProject } from "../../core/ops/project-ops";
import { loadTeams } from "../../core/ops/team-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { hasErrors, requiredError, slugError } from "../../core/primitives/forms";
import {
  busyForm,
  doneForm,
  failedForm,
  failedLoad,
  idleForm,
  loaded,
  loading,
} from "../../core/primitives/load-status";
import { ProjectActions, type ProjectEdit, ProjectState } from "./project-model";

export type ProjectDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

export interface ProjectRouting {
  onDeleted(): void;
}

/** One project: details, linked teams, delete. Linking a team needs managing both the project and the team. */
export class ProjectController extends SubtreeController implements ProjectActions {
  readonly state = new ProjectState();
  #teams: readonly TeamSummary[] = [];

  constructor(
    readonly deps: ProjectDeps,
    readonly routing: ProjectRouting,
    readonly orgId: number,
    readonly projectId: number,
  ) {
    super();
    this.subtree.put(ProjectState, this.state);
    this.subtree.put(ProjectActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const [project, teams] = await Promise.all([
      loadProject({ ...this.deps, projectId: this.projectId }),
      loadTeams({ ...this.deps, orgId: this.orgId }),
    ]);
    if (this.isDisposed) return;
    if (!project.ok || !teams.ok) {
      this.state.load.value = failedLoad([...(project.ok ? [] : project.errors), ...(teams.ok ? [] : teams.errors)]);
      return;
    }
    this.#teams = teams.value;
    this.#show(project.value);
    this.state.draft.value = { name: project.value.name, slug: project.value.slug };
    this.state.load.value = loaded;
  };

  edit = (patch: Partial<ProjectEdit>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    this.state.fieldErrors.value = {};
    this.state.save.value = idleForm;
  };

  saveDetails = async () => {
    if (this.state.save.value.busy) return;
    const draft = this.state.draft.value;
    const errors = {
      name: requiredError(draft.name, "a project name"),
      slug: requiredError(draft.slug, "a slug") ?? slugError(draft.slug.trim()),
    };
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.save.value = busyForm;
    const r = await updateProject({ ...this.deps, projectId: this.projectId, update: draft });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.fieldErrors.value = fieldErrors(r.errors);
      const rest = formLevelErrors(r.errors);
      this.state.save.value = rest.length > 0 ? failedForm(rest) : idleForm;
      return;
    }
    this.#show(r.value);
    this.state.save.value = doneForm("Saved.");
  };

  linkTeam = (teamId: number) =>
    this.#act(async () => {
      const r = await linkTeam({ ...this.deps, projectId: this.projectId, teamId });
      return r.ok ? this.#refresh() : r;
    });

  unlinkTeam = (teamId: number) =>
    this.#act(async () => {
      const r = await unlinkTeam({ ...this.deps, projectId: this.projectId, teamId });
      return r.ok ? this.#refresh() : r;
    });

  deleteProject = () =>
    this.#act(async () => {
      const r = await deleteProject({ ...this.deps, projectId: this.projectId });
      if (r.ok) this.routing.onDeleted();
      return r;
    });

  /** After a link change: whether a manager still manages the project may have changed with it. */
  async #refresh(): Promise<{ ok: boolean; errors?: readonly Error[] }> {
    const r = await loadProject({ ...this.deps, projectId: this.projectId });
    if (r.ok && !this.isDisposed) this.#show(r.value);
    return r;
  }

  #show(project: Project): void {
    this.state.project.value = project;
    const linked = new Set(project.teams.map((t) => t.id));
    this.state.linkable.value = project.canManage ? this.#teams.filter((t) => t.canManage && !linked.has(t.id)) : [];
  }

  async #act(fn: () => Promise<{ ok: boolean; errors?: readonly Error[] }>): Promise<void> {
    if (this.state.busy.value) return;
    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await fn();
      if (!this.isDisposed && !r.ok) this.state.error.value = errorText(r.errors ?? []);
    } finally {
      if (!this.isDisposed) this.state.busy.value = false;
    }
  }
}
