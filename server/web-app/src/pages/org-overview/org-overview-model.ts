import type { Organization, Project, TeamSummary } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";
import { idleLoad, type LoadStatus } from "../../core/primitives/load-status";

export class OrgOverviewState {
  readonly org = new Rx<Organization | null>(null);
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly teams = new RxList<TeamSummary>();
  readonly projects = new RxList<Project>();
  readonly memberCount = new Rx(0);
}

export abstract class OrgOverviewActions {
  abstract reload(): Promise<void>;
}
