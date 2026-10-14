import type { Account, MyOrganization } from "@simpled/backend-client";
import { Rx, RxList } from "subtree.js/core";

export class NewOrgState {
  readonly name = new Rx("");
  readonly nameError = new Rx<string | null>(null);
  readonly error = new Rx<string | null>(null);
  readonly busy = new Rx(false);
  readonly account = new Rx<Account | null>(null);
  readonly organizations = new RxList<MyOrganization>();
}

export abstract class NewOrgActions {
  abstract setName(value: string): void;
  abstract submit(): Promise<void>;
  abstract signOut(): Promise<void>;
}
