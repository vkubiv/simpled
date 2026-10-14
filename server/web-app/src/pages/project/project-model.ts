import type { Organization, Project, TeamSummary } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export interface ProjectEdit {
  readonly name: string;
  readonly slug: string;
}

export class ProjectState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly project = new Rx<Project | null>(null);
  /** Teams the caller may link and that are not linked yet. */
  readonly linkable = new RxList<TeamSummary>();
  readonly draft = new Rx<ProjectEdit>({ name: "", slug: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof ProjectEdit>>({});
  readonly save = new Rx<FormStatus>(idleForm);
  readonly busy = new Rx(false);
  readonly error = new Rx<string | null>(null);
}

export abstract class ProjectActions {
  abstract reload(): Promise<void>;
  abstract edit(patch: Partial<ProjectEdit>): void;
  abstract saveDetails(): Promise<void>;
  abstract linkTeam(teamId: number): Promise<void>;
  abstract unlinkTeam(teamId: number): Promise<void>;
  abstract deleteProject(): Promise<void>;
}
