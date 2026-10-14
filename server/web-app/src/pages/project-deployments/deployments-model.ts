import type { DeploymentEvent, GitHubConnection, Organization, Repository } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export interface LinkDraft {
  /** A connection id from the select, "" before one is picked. */
  readonly connectionId: string;
  readonly fullName: string;
}

/** The history shown under one deployment. */
export interface OpenEvents {
  readonly deploymentId: number;
  readonly load: LoadStatus;
  readonly events: readonly DeploymentEvent[];
}

export class DeploymentsState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly repositories = new RxList<Repository>();
  /** Whether the caller manages the project: link, sync and unlink. */
  readonly canManage = new Rx(false);
  /** The organization's connections, offered when linking. */
  readonly connections = new RxList<GitHubConnection>();
  readonly draft = new Rx<LinkDraft>({ connectionId: "", fullName: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof LinkDraft>>({});
  readonly link = new Rx<FormStatus>(idleForm);
  readonly busy = new Rx(false);
  readonly error = new Rx<string | null>(null);
  readonly openEvents = new Rx<OpenEvents | null>(null);
  /** When the statuses were last read; they refresh on their own while the page is open. */
  readonly refreshedAt = new Rx<Date | null>(null);
}

export abstract class DeploymentsActions {
  abstract reload(): Promise<void>;
  abstract refresh(): Promise<void>;
  abstract editLink(patch: Partial<LinkDraft>): void;
  abstract linkRepository(): Promise<void>;
  abstract syncRepository(repositoryId: number): Promise<void>;
  abstract unlinkRepository(repositoryId: number): Promise<void>;
  abstract toggleEvents(deploymentId: number): Promise<void>;
}
