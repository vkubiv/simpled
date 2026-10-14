import { Rx } from "subtree.js/core";
import { type FormStatus, idleForm } from "../../core/primitives/load-status";

export class ForgotPasswordState {
  readonly email = new Rx("");
  readonly emailError = new Rx<string | null>(null);
  readonly status = new Rx<FormStatus>(idleForm);
}

export abstract class ForgotPasswordActions {
  abstract setEmail(value: string): void;
  abstract submit(): Promise<void>;
}
