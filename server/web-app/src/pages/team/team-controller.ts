import type { Team } from "@simpled/backend-client";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { loadMembers } from "../../core/ops/org-ops";
import { addTeamMember, deleteTeam, loadTeam, removeTeamMember, renameTeam } from "../../core/ops/team-ops";
import { errorText, fieldErrors } from "../../core/primitives/errors";
import { requiredError } from "../../core/primitives/forms";
import { busyForm, doneForm, failedForm, failedLoad, loaded, loading } from "../../core/primitives/load-status";
import { TeamActions, TeamState } from "./team-model";

export type TeamDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

export interface TeamRouting {
  onDeleted(): void;
}

/** One team: its members and projects; renaming, membership and deleting for whoever manages it. */
export class TeamController extends SubtreeController implements TeamActions {
  readonly state = new TeamState();

  constructor(
    readonly deps: TeamDeps,
    readonly routing: TeamRouting,
    readonly orgId: number,
    readonly teamId: number,
  ) {
    super();
    this.subtree.put(TeamState, this.state);
    this.subtree.put(TeamActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const r = await loadTeam({ ...this.deps, teamId: this.teamId });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.load.value = failedLoad(r.errors);
      return;
    }
    this.state.name.value = r.value.name;
    await this.#show(r.value);
    this.state.load.value = loaded;
  };

  setName = (value: string) => {
    this.state.name.value = value;
    this.state.rename.value = { busy: false, error: null, done: null };
  };

  saveName = async () => {
    if (this.state.rename.value.busy) return;
    const name = this.state.name.value;
    const error = requiredError(name, "a team name");
    if (error) {
      this.state.rename.value = failedForm(error);
      return;
    }
    this.state.rename.value = busyForm;
    const r = await renameTeam({ ...this.deps, teamId: this.teamId, name });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.rename.value = failedForm(fieldErrors(r.errors).name ?? errorText(r.errors));
      return;
    }
    this.state.team.value = r.value;
    this.state.rename.value = doneForm("Saved.");
  };

  addMember = (accountId: string) =>
    this.#act(async () => {
      const r = await addTeamMember({ ...this.deps, teamId: this.teamId, accountId });
      return r.ok ? this.#refresh() : r;
    });

  removeMember = (accountId: string) =>
    this.#act(async () => {
      const r = await removeTeamMember({ ...this.deps, teamId: this.teamId, accountId });
      return r.ok ? this.#refresh() : r;
    });

  deleteTeam = () =>
    this.#act(async () => {
      const r = await deleteTeam({ ...this.deps, teamId: this.teamId });
      if (r.ok) this.routing.onDeleted();
      return r;
    });

  /** After a membership change: whether the caller still manages the team may have changed with it. */
  async #refresh(): Promise<{ ok: boolean; errors?: readonly Error[] }> {
    const r = await loadTeam({ ...this.deps, teamId: this.teamId });
    if (r.ok && !this.isDisposed) await this.#show(r.value);
    return r;
  }

  async #show(team: Team): Promise<void> {
    this.state.team.value = team;
    if (!team.canManage) {
      this.state.candidates.value = [];
      return;
    }
    const members = await loadMembers({ ...this.deps, orgId: this.orgId });
    if (this.isDisposed || !members.ok) return;
    const inTeam = new Set(team.members.map((m) => m.accountId));
    this.state.candidates.value = members.value.filter((m) => !inTeam.has(m.accountId));
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
