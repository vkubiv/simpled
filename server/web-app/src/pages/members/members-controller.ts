import type { OrgRole } from "@simpled/backend-client";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { refreshMe } from "../../core/ops/auth-ops";
import { invite, loadInvitations, revokeInvitation } from "../../core/ops/invitation-ops";
import { loadMembers, loadOrg, removeMember, setMemberRole } from "../../core/ops/org-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { emailError } from "../../core/primitives/forms";
import {
  busyForm,
  doneForm,
  failedForm,
  failedLoad,
  idleForm,
  loaded,
  loading,
} from "../../core/primitives/load-status";
import { type InviteDraft, MembersActions, MembersState } from "./members-model";

export type MembersDeps = Pick<AppDeps, "client" | "session" | "authHandler" | "meRepository" | "orgRepository">;

export interface MembersRouting {
  /** The caller left the organization. */
  onLeft(): void;
}

/** Members with their roles, and for owners and admins the invitations. What each row allows comes from main-svc. */
export class MembersController extends SubtreeController implements MembersActions {
  readonly state = new MembersState();

  constructor(
    readonly deps: MembersDeps,
    readonly routing: MembersRouting,
    readonly orgId: number,
  ) {
    super();
    this.subtree.put(MembersState, this.state);
    this.subtree.put(MembersActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    void this.reload();
  }

  reload = async () => {
    this.state.load.value = loading;
    const o = { ...this.deps, orgId: this.orgId };
    const canManage = this.deps.orgRepository.org?.permissions.canManageMembers ?? false;
    const [members, invitations] = await Promise.all([loadMembers(o), canManage ? loadInvitations(o) : null]);
    if (this.isDisposed) return;
    if (!members.ok) {
      this.state.load.value = failedLoad(members.errors);
      return;
    }
    if (invitations && !invitations.ok) {
      this.state.load.value = failedLoad(invitations.errors);
      return;
    }
    this.state.members.value = members.value;
    this.state.invitations.value = invitations?.value ?? [];
    this.state.load.value = loaded;
  };

  editInvite = (patch: Partial<InviteDraft>) => {
    this.state.invite.update((d) => ({ ...d, ...patch }));
    this.state.inviteEmailError.value = null;
    if (!this.state.inviteStatus.value.busy) this.state.inviteStatus.value = idleForm;
  };

  sendInvite = async () => {
    if (this.state.inviteStatus.value.busy) return;
    const draft = this.state.invite.value;
    const error = emailError(draft.email);
    this.state.inviteEmailError.value = error ?? null;
    if (error) return;

    this.state.inviteStatus.value = busyForm;
    const r = await invite({ ...this.deps, orgId: this.orgId, email: draft.email, role: draft.role });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.inviteEmailError.value = fieldErrors(r.errors).email ?? null;
      const rest = formLevelErrors(r.errors);
      this.state.inviteStatus.value = rest.length > 0 ? failedForm(rest) : idleForm;
      return;
    }
    this.state.invite.update((d) => ({ ...d, email: "" }));
    this.state.invitations.update((list) => [r.value, ...list.filter((i) => i.email !== r.value.email)]);
    this.state.inviteStatus.value = doneForm(`Invitation sent to ${r.value.email}.`);
  };

  changeRole = (accountId: string, role: OrgRole) =>
    this.#row(accountId, async () => {
      const isMe = this.state.members.value.find((m) => m.accountId === accountId)?.isMe ?? false;
      const r = await setMemberRole({ ...this.deps, orgId: this.orgId, accountId, role });
      if (!r.ok) return r;
      // A changed role of one's own changes what this page and the shell allow.
      if (isMe) await Promise.all([loadOrg({ ...this.deps, orgId: this.orgId }), refreshMe(this.deps)]);
      await this.#refresh();
      return r;
    });

  remove = (accountId: string) =>
    this.#row(accountId, async () => {
      const isMe = this.state.members.value.find((m) => m.accountId === accountId)?.isMe ?? false;
      const r = await removeMember({ ...this.deps, orgId: this.orgId, accountId, isMe });
      if (r.ok && isMe) this.routing.onLeft();
      else if (r.ok) this.state.members.update((list) => list.filter((m) => m.accountId !== accountId));
      return r;
    });

  revoke = (invitationId: number) =>
    this.#row(`invitation:${invitationId}`, async () => {
      const r = await revokeInvitation({ ...this.deps, orgId: this.orgId, invitationId });
      if (r.ok) this.state.invitations.update((list) => list.filter((i) => i.id !== invitationId));
      return r;
    });

  /** Rows' flags (who may change whom) depend on everyone's roles, so a role change reloads the list. */
  async #refresh(): Promise<void> {
    const r = await loadMembers({ ...this.deps, orgId: this.orgId });
    if (r.ok && !this.isDisposed) this.state.members.value = r.value;
  }

  async #row(key: string, fn: () => Promise<{ ok: boolean; errors?: readonly Error[] }>): Promise<void> {
    if (this.state.busyRow.value !== null) return;
    this.state.busyRow.value = key;
    this.state.rowError.value = null;
    try {
      const r = await fn();
      if (!this.isDisposed && !r.ok) this.state.rowError.value = errorText(r.errors ?? []);
    } finally {
      if (!this.isDisposed) this.state.busyRow.value = null;
    }
  }
}
