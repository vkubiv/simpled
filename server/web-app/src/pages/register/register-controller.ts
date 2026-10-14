import { AccountAlreadyExists, type Registration } from "@simpled/backend-client";
import { getError } from "operation-result.js";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { register } from "../../core/ops/auth-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { emailError, hasErrors, newPasswordError, requiredError } from "../../core/primitives/forms";
import { RegisterActions, RegisterState } from "./register-model";

export type RegisterDeps = Pick<AppDeps, "client" | "session">;

export interface RegisterRouting {
  /** The account exists but is unconfirmed: the person goes to their inbox. */
  onRegistered(email: string): void;
}

export class RegisterController extends SubtreeController implements RegisterActions {
  readonly state = new RegisterState();
  #submitted = false;

  constructor(
    readonly deps: RegisterDeps,
    readonly routing: RegisterRouting,
  ) {
    super();
    this.subtree.put(RegisterState, this.state);
    this.subtree.put(RegisterActions, this);
  }

  edit = (patch: Partial<Registration>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    if (this.#submitted) this.state.fieldErrors.value = validate(this.state.draft.value);
  };

  submit = async () => {
    if (this.state.busy.value) return;
    this.#submitted = true;
    const registration = this.state.draft.value;
    const errors = validate(registration);
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await register({ ...this.deps, registration });
      if (this.isDisposed) return;
      if (r.ok) {
        this.routing.onRegistered(registration.email.trim());
        return;
      }
      const taken = getError(r, AccountAlreadyExists);
      this.state.fieldErrors.value = { ...fieldErrors(r.errors), ...(taken ? { email: taken.message } : {}) };
      const rest = formLevelErrors(r.errors).filter((e) => e !== taken);
      if (rest.length > 0) this.state.error.value = errorText(rest);
    } finally {
      this.state.busy.value = false;
    }
  };
}

function validate(r: Registration) {
  return {
    name: requiredError(r.name, "your name"),
    email: emailError(r.email),
    password: newPasswordError(r.password, r.email),
    organizationName: requiredError(r.organizationName, "an organization name"),
  };
}
