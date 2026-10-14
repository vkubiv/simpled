import type { Member, Organization, Team } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export class TeamState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly team = new Rx<Team | null>(null);
  /** Organization members not in the team yet, for "add member"; loaded only when the caller manages the team. */
  readonly candidates = new RxList<Member>();
  readonly name = new Rx("");
  readonly rename = new Rx<FormStatus>(idleForm);
  readonly busy = new Rx(false);
  readonly error = new Rx<string | null>(null);
}

export abstract class TeamActions {
  abstract reload(): Promise<void>;
  abstract setName(value: string): void;
  abstract saveName(): Promise<void>;
  abstract addMember(accountId: string): Promise<void>;
  abstract removeMember(accountId: string): Promise<void>;
  abstract deleteTeam(): Promise<void>;
}
