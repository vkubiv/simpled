import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { createTeam, loadTeams } from "../../core/ops/team-ops";
import { fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { requiredError } from "../../core/primitives/forms";
import { busyForm, failedForm, failedLoad, idleForm, loaded, loading } from "../../core/primitives/load-status";
import { TeamsActions, TeamsState } from "./teams-model";

export type TeamsDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

export interface TeamsRouting {
  onCreated(teamId: number): void;
}

export class TeamsController extends SubtreeController implements TeamsActions {
  readonly state = new TeamsState();

  constructor(
    readonly deps: TeamsDeps,
    readonly routing: TeamsRouting,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(TeamsState, this.state);
    this.subtree.put(TeamsActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const r = await loadTeams({ ...this.deps, orgId: this.orgId });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.load.value = failedLoad(r.errors);
      return;
    }
    this.state.teams.value = r.value;
    this.state.load.value = loaded;
  };

  setNewName = (value: string) => {
    this.state.newName.value = value;
    this.state.newNameError.value = null;
  };

  /** A manager who creates a team is in it, so they can manage it; the new team's page opens. */
  createTeam = async () => {
    if (this.state.create.value.busy) return;
    const name = this.state.newName.value;
    const error = requiredError(name, "a team name");
    this.state.newNameError.value = error ?? null;
    if (error) return;

    this.state.create.value = busyForm;
    const r = await createTeam({ ...this.deps, orgId: this.orgId, name });
    if (this.isDisposed) return;
    if (r.ok) {
      this.state.create.value = idleForm;
      this.routing.onCreated(r.value.id);
      return;
    }
    this.state.newNameError.value = fieldErrors(r.errors).name ?? null;
    const rest = formLevelErrors(r.errors);
    this.state.create.value = rest.length > 0 ? failedForm(rest) : idleForm;
  };
}
