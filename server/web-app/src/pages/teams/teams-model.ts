import type { Organization, TeamSummary } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export class TeamsState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly teams = new RxList<TeamSummary>();
  readonly newName = new Rx("");
  readonly newNameError = new Rx<string | null>(null);
  readonly create = new Rx<FormStatus>(idleForm);
}

export abstract class TeamsActions {
  abstract reload(): Promise<void>;
  abstract setNewName(value: string): void;
  abstract createTeam(): Promise<void>;
}
