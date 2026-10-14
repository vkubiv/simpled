import type { GitHubConnection } from "@simpled/backend-client";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { createConnection, deleteConnection, loadConnections, updateConnection } from "../../core/ops/monitoring-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { hasErrors, requiredError } from "../../core/primitives/forms";
import {
  busyForm,
  doneForm,
  failedForm,
  failedLoad,
  idleForm,
  loaded,
  loading,
} from "../../core/primitives/load-status";
import { type ConnectionDraft, GitHubActions, GitHubState } from "./github-model";

export type GitHubDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

/** The organization's GitHub tokens: everyone sees them, owners and admins add, rotate and remove them. */
export class GitHubController extends SubtreeController implements GitHubActions {
  readonly state = new GitHubState();

  constructor(
    readonly deps: GitHubDeps,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(GitHubState, this.state);
    this.subtree.put(GitHubActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const r = await loadConnections({ ...this.deps, orgId: this.orgId });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.load.value = failedLoad(r.errors);
      return;
    }
    this.state.connections.value = r.value;
    this.state.load.value = loaded;
  };

  edit = (patch: Partial<ConnectionDraft>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    this.state.fieldErrors.value = {};
    this.state.create.value = idleForm;
  };

  addConnection = async () => {
    if (this.state.create.value.busy) return;
    const draft = this.state.draft.value;
    const errors = {
      name: requiredError(draft.name, "a name"),
      token: requiredError(draft.token, "a token"),
    };
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.create.value = busyForm;
    const r = await createConnection({ ...this.deps, orgId: this.orgId, connection: draft });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.fieldErrors.value = fieldErrors(r.errors);
      const rest = formLevelErrors(r.errors);
      this.state.create.value = rest.length > 0 ? failedForm(rest) : idleForm;
      return;
    }
    this.#put(r.value);
    this.state.draft.value = { name: "", token: "" };
    this.state.create.value = doneForm(`Connected as ${r.value.gitHubLogin}.`);
  };

  startEditing = (connectionId: number) => {
    const connection = this.state.connections.value.find((c) => c.id === connectionId);
    if (!connection) return;
    this.state.editing.value = { connectionId, name: connection.name, token: "" };
    this.state.editErrors.value = {};
    this.state.save.value = idleForm;
  };

  editConnection = (patch: Partial<ConnectionDraft>) => {
    this.state.editing.update((e) => (e ? { ...e, ...patch } : e));
    this.state.editErrors.value = {};
    this.state.save.value = idleForm;
  };

  cancelEditing = () => {
    this.state.editing.value = null;
  };

  saveConnection = async () => {
    const editing = this.state.editing.value;
    if (!editing || this.state.save.value.busy) return;
    const errors = { name: requiredError(editing.name, "a name") };
    this.state.editErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.save.value = busyForm;
    const r = await updateConnection({
      ...this.deps,
      connectionId: editing.connectionId,
      update: { name: editing.name, token: editing.token },
    });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.editErrors.value = fieldErrors(r.errors);
      const rest = formLevelErrors(r.errors);
      this.state.save.value = rest.length > 0 ? failedForm(rest) : idleForm;
      return;
    }
    this.#put(r.value);
    this.state.editing.value = null;
    this.state.save.value = idleForm;
  };

  deleteConnection = async (connectionId: number) => {
    if (this.state.busy.value) return;
    this.state.busy.value = true;
    this.state.error.value = null;
    const r = await deleteConnection({ ...this.deps, connectionId });
    if (this.isDisposed) return;
    this.state.busy.value = false;
    if (!r.ok) {
      this.state.error.value = errorText(r.errors);
      return;
    }
    this.state.connections.value = this.state.connections.value.filter((c) => c.id !== connectionId);
  };

  #put(connection: GitHubConnection): void {
    const others = this.state.connections.value.filter((c) => c.id !== connection.id);
    this.state.connections.value = [...others, connection].sort((a, b) => a.name.localeCompare(b.name));
  }
}
