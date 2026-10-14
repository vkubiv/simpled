import type { Account, MyOrganization, Organization } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";

export type ShellView = "loading" | "ready" | "notFound" | "failed";

export class ShellState {
  readonly orgId: number;
  readonly view = new Rx<ShellView>("loading");
  readonly org = new Rx<Organization | null>(null);
  readonly failure = new Rx<string | null>(null);
  readonly account = new Rx<Account | null>(null);
  /** For the switcher. */
  readonly organizations = new RxList<MyOrganization>();

  constructor(orgId: number) {
    this.orgId = orgId;
  }
}

export abstract class ShellActions {
  abstract signOut(): Promise<void>;
  abstract retry(): Promise<void>;
}
