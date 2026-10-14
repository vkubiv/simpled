import type { Invitation, Member, Organization, OrgRole } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export interface InviteDraft {
  readonly email: string;
  readonly role: OrgRole;
}

export class MembersState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly members = new RxList<Member>();
  /** Open invitations; only owners and admins see them. */
  readonly invitations = new RxList<Invitation>();
  readonly invite = new Rx<InviteDraft>({ email: "", role: "member" });
  readonly inviteEmailError = new Rx<string | null>(null);
  readonly inviteStatus = new Rx<FormStatus>(idleForm);
  /** The row a change is in flight for: an account id, or `invitation:<id>`. */
  readonly busyRow = new Rx<string | null>(null);
  /** The last refused row change, shown above the list. */
  readonly rowError = new Rx<string | null>(null);
}

export abstract class MembersActions {
  abstract reload(): Promise<void>;
  abstract editInvite(patch: Partial<InviteDraft>): void;
  abstract sendInvite(): Promise<void>;
  abstract changeRole(accountId: string, role: OrgRole): Promise<void>;
  abstract remove(accountId: string): Promise<void>;
  abstract revoke(invitationId: number): Promise<void>;
}
