import type { Organization } from "@simpled/backend-client";
import { Rx } from "subtree.js/core";
import { type FormStatus, idleForm } from "../../core/primitives/load-status";

export class OrgSettingsState {
  readonly org = new Rx<Organization | null>(null);
  readonly name = new Rx("");
  readonly rename = new Rx<FormStatus>(idleForm);
  /** The organization's name, typed again to enable deleting it. */
  readonly deleteConfirmation = new Rx("");
  readonly canDelete = new Rx(false);
  readonly danger = new Rx<FormStatus>(idleForm);
}

export abstract class OrgSettingsActions {
  abstract setName(value: string): void;
  abstract saveName(): Promise<void>;
  abstract setDeleteConfirmation(value: string): void;
  abstract deleteOrg(): Promise<void>;
  abstract leave(): Promise<void>;
}
