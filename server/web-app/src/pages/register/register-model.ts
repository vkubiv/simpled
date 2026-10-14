import type { Registration } from "@simpled/backend-client";
import { Rx } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";

export class RegisterState {
  readonly draft = new Rx<Registration>({ name: "", email: "", password: "", organizationName: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof Registration>>({});
  readonly error = new Rx<string | null>(null);
  readonly busy = new Rx(false);
}

export abstract class RegisterActions {
  abstract edit(patch: Partial<Registration>): void;
  abstract submit(): Promise<void>;
}
