import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { requestPasswordReset } from "../../core/ops/auth-ops";
import { emailError } from "../../core/primitives/forms";
import { busyForm, doneForm, failedForm } from "../../core/primitives/load-status";
import { ForgotPasswordActions, ForgotPasswordState } from "./forgot-password-model";

export type ForgotPasswordDeps = Pick<AppDeps, "client" | "session">;

/** main-svc answers the same whether or not the account exists, and so does this page. */
export class ForgotPasswordController extends SubtreeController implements ForgotPasswordActions {
  readonly state = new ForgotPasswordState();

  constructor(readonly deps: ForgotPasswordDeps) {
    super();
    this.subtree.put(ForgotPasswordState, this.state);
    this.subtree.put(ForgotPasswordActions, this);
  }

  setEmail = (value: string) => {
    this.state.email.value = value;
    this.state.emailError.value = null;
  };

  submit = async () => {
    if (this.state.status.value.busy) return;
    const email = this.state.email.value;
    const error = emailError(email);
    this.state.emailError.value = error ?? null;
    if (error) return;
    this.state.status.value = busyForm;
    const r = await requestPasswordReset({ ...this.deps, email });
    if (this.isDisposed) return;
    this.state.status.value = r.ok
      ? doneForm(`If ${email.trim()} has an account, a reset link is on its way. It is valid for one hour.`)
      : failedForm(r.errors);
  };
}
