import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { resetPassword } from "../../core/ops/auth-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { newPasswordError } from "../../core/primitives/forms";
import { ResetPasswordActions, ResetPasswordState } from "./reset-password-model";

export type ResetPasswordDeps = Pick<AppDeps, "client" | "session">;

export interface ResetPasswordRouting {
  /** Every session was signed out; the person signs in with the new password. */
  onReset(): void;
}

export class ResetPasswordController extends SubtreeController implements ResetPasswordActions {
  readonly state = new ResetPasswordState();

  constructor(
    readonly deps: ResetPasswordDeps,
    readonly routing: ResetPasswordRouting,
    readonly token: string,
  ) {
    super();
    this.subtree.put(ResetPasswordState, this.state);
    this.subtree.put(ResetPasswordActions, this);
    if (!token) this.state.error.value = "The link is incomplete. Open it again from the email.";
  }

  setPassword = (value: string) => {
    this.state.password.value = value;
    this.state.passwordError.value = null;
  };

  submit = async () => {
    if (this.state.busy.value || !this.token) return;
    const password = this.state.password.value;
    const error = newPasswordError(password);
    this.state.passwordError.value = error ?? null;
    if (error) return;

    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await resetPassword({ ...this.deps, token: this.token, newPassword: password });
      if (this.isDisposed) return;
      if (r.ok) {
        this.routing.onReset();
        return;
      }
      this.state.passwordError.value = fieldErrors(r.errors).password ?? null;
      const rest = formLevelErrors(r.errors);
      if (rest.length > 0) this.state.error.value = errorText(rest);
    } finally {
      this.state.busy.value = false;
    }
  };
}
