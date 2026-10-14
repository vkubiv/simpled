import { Rx } from "subtree.js/core";
import { type FormStatus, idleForm } from "../../core/primitives/load-status";

export class CheckEmailState {
  readonly email: string;
  readonly resend = new Rx<FormStatus>(idleForm);

  constructor(email: string) {
    this.email = email;
  }
}

export abstract class CheckEmailActions {
  abstract resend(): Promise<void>;
}
