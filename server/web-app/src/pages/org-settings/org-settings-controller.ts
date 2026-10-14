import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { deleteOrg, removeMember, renameOrg } from "../../core/ops/org-ops";
import { errorText, fieldErrors } from "../../core/primitives/errors";
import { requiredError } from "../../core/primitives/forms";
import { busyForm, doneForm, failedForm, idleForm } from "../../core/primitives/load-status";
import { OrgSettingsActions, OrgSettingsState } from "./org-settings-model";

export type OrgSettingsDeps = Pick<AppDeps, "client" | "session" | "authHandler" | "meRepository" | "orgRepository">;

export interface OrgSettingsRouting {
  /** The organization was deleted, or the caller left it. */
  onGone(): void;
}

/** Renaming and deleting (owners), and leaving (anyone but the last owner). */
export class OrgSettingsController extends SubtreeController implements OrgSettingsActions {
  readonly state = new OrgSettingsState();

  constructor(
    readonly deps: OrgSettingsDeps,
    readonly routing: OrgSettingsRouting,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(OrgSettingsState, this.state);
    this.subtree.put(OrgSettingsActions, this);
    this.state.name.value = deps.orgRepository.org?.name ?? "";
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    this.sync(() => {
      const org = this.state.org.value;
      this.state.canDelete.value = org !== null && this.state.deleteConfirmation.value.trim() === org.name;
    }, [this.state.org, this.state.deleteConfirmation]);
  }

  setName = (value: string) => {
    this.state.name.value = value;
    this.state.rename.value = idleForm;
  };

  saveName = async () => {
    if (this.state.rename.value.busy) return;
    const name = this.state.name.value;
    const error = requiredError(name, "a name");
    if (error) {
      this.state.rename.value = failedForm(error);
      return;
    }
    this.state.rename.value = busyForm;
    const r = await renameOrg({ ...this.deps, orgId: this.orgId, name });
    if (this.isDisposed) return;
    this.state.rename.value = r.ok ? doneForm("Saved.") : failedForm(fieldErrors(r.errors).name ?? errorText(r.errors));
  };

  setDeleteConfirmation = (value: string) => {
    this.state.deleteConfirmation.value = value;
  };

  deleteOrg = async () => {
    if (!this.state.canDelete.value || this.state.danger.value.busy) return;
    this.state.danger.value = busyForm;
    const r = await deleteOrg({ ...this.deps, orgId: this.orgId });
    if (this.isDisposed) return;
    if (r.ok) this.routing.onGone();
    else this.state.danger.value = failedForm(r.errors);
  };

  leave = async () => {
    const me = this.deps.meRepository.me;
    if (!me || this.state.danger.value.busy) return;
    this.state.danger.value = busyForm;
    const r = await removeMember({ ...this.deps, orgId: this.orgId, accountId: me.account.id, isMe: true });
    if (this.isDisposed) return;
    if (r.ok) this.routing.onGone();
    else this.state.danger.value = failedForm(r.errors);
  };
}
