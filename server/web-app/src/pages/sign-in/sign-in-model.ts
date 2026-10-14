import { Rx } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";

export interface SignInDraft {
  readonly email: string;
  readonly password: string;
}

export class SignInState {
  readonly draft = new Rx<SignInDraft>({ email: "", password: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof SignInDraft>>({});
  readonly error = new Rx<string | null>(null);
  /** Set when the account exists but its email is not confirmed: the page offers a new link. */
  readonly unconfirmedEmail = new Rx<string | null>(null);
  readonly info = new Rx<string | null>(null);
  readonly busy = new Rx(false);
}

export abstract class SignInActions {
  abstract setEmail(value: string): void;
  abstract setPassword(value: string): void;
  abstract submit(): Promise<void>;
}
