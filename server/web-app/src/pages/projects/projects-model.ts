import type { Organization, Project, TeamSummary } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export interface ProjectDraft {
  readonly name: string;
  /** Empty: main-svc makes one from the name. */
  readonly slug: string;
  readonly teamIds: readonly number[];
}

export class ProjectsState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly projects = new RxList<Project>();
  /** Teams a new project may be linked to: all for owners and admins, their own for a manager. */
  readonly teamOptions = new RxList<TeamSummary>();
  readonly draft = new Rx<ProjectDraft>({ name: "", slug: "", teamIds: [] });
  readonly fieldErrors = new Rx<FieldErrors<keyof ProjectDraft>>({});
  readonly create = new Rx<FormStatus>(idleForm);
}

export abstract class ProjectsActions {
  abstract reload(): Promise<void>;
  abstract edit(patch: Partial<Omit<ProjectDraft, "teamIds">>): void;
  abstract toggleTeam(teamId: number): void;
  abstract createProject(): Promise<void>;
}
