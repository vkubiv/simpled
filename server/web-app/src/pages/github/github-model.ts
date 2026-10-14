import type { GitHubConnection, Organization } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export interface ConnectionDraft {
  readonly name: string;
  readonly token: string;
}

/** A connection being renamed or given a new token. */
export interface ConnectionEdit extends ConnectionDraft {
  readonly connectionId: number;
}

export class GitHubState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly connections = new RxList<GitHubConnection>();
  readonly draft = new Rx<ConnectionDraft>({ name: "", token: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof ConnectionDraft>>({});
  readonly create = new Rx<FormStatus>(idleForm);
  readonly editing = new Rx<ConnectionEdit | null>(null);
  readonly editErrors = new Rx<FieldErrors<keyof ConnectionDraft>>({});
  readonly save = new Rx<FormStatus>(idleForm);
  readonly busy = new Rx(false);
  readonly error = new Rx<string | null>(null);
}

export abstract class GitHubActions {
  abstract reload(): Promise<void>;
  abstract edit(patch: Partial<ConnectionDraft>): void;
  abstract addConnection(): Promise<void>;
  abstract startEditing(connectionId: number): void;
  abstract editConnection(patch: Partial<ConnectionDraft>): void;
  abstract cancelEditing(): void;
  abstract saveConnection(): Promise<void>;
  abstract deleteConnection(connectionId: number): Promise<void>;
}
