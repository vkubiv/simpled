import { Rx } from "subtree.js/core";

export class ResetPasswordState {
  readonly password = new Rx("");
  readonly passwordError = new Rx<string | null>(null);
  readonly error = new Rx<string | null>(null);
  readonly busy = new Rx(false);
}

export abstract class ResetPasswordActions {
  abstract setPassword(value: string): void;
  abstract submit(): Promise<void>;
}
