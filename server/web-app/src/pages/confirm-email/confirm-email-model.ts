import { Rx } from "subtree.js/core";

export class ConfirmEmailState {
  readonly busy = new Rx(false);
  readonly error = new Rx<string | null>(null);
}

export abstract class ConfirmEmailActions {
  abstract confirm(): Promise<void>;
}
