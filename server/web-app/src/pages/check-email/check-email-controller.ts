import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { resendConfirmation } from "../../core/ops/auth-ops";
import { busyForm, doneForm, failedForm } from "../../core/primitives/load-status";
import { CheckEmailActions, CheckEmailState } from "./check-email-model";

export type CheckEmailDeps = Pick<AppDeps, "client" | "session">;

/** After registering, or signing in unconfirmed: the inbox is the next step, and a new link is one click. */
export class CheckEmailController extends SubtreeController implements CheckEmailActions {
  readonly state: CheckEmailState;

  constructor(
    readonly deps: CheckEmailDeps,
    email: string,
  ) {
    super();
    this.state = new CheckEmailState(email);
    this.subtree.put(CheckEmailState, this.state);
    this.subtree.put(CheckEmailActions, this);
  }

  resend = async () => {
    if (this.state.resend.value.busy || !this.state.email) return;
    this.state.resend.value = busyForm;
    const r = await resendConfirmation({ ...this.deps, email: this.state.email });
    if (this.isDisposed) return;
    this.state.resend.value = r.ok
      ? doneForm("We sent a new link. Earlier links no longer work.")
      : failedForm(r.errors);
  };
}
