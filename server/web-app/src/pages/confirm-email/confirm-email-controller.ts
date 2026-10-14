import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { confirmEmail } from "../../core/ops/auth-ops";
import { errorText } from "../../core/primitives/errors";
import { ConfirmEmailActions, ConfirmEmailState } from "./confirm-email-model";

export type ConfirmEmailDeps = Pick<AppDeps, "client" | "session" | "meRepository">;

export interface ConfirmEmailRouting {
  /** Confirmed and signed in. */
  onConfirmed(): void;
}

/**
 * Confirms on a click, not on load: mail scanners open links before the person does, and the token is single use.
 */
export class ConfirmEmailController extends SubtreeController implements ConfirmEmailActions {
  readonly state = new ConfirmEmailState();

  constructor(
    readonly deps: ConfirmEmailDeps,
    readonly routing: ConfirmEmailRouting,
    readonly token: string,
  ) {
    super();
    this.subtree.put(ConfirmEmailState, this.state);
    this.subtree.put(ConfirmEmailActions, this);
    if (!token) this.state.error.value = "The link is incomplete. Open it again from the email.";
  }

  confirm = async () => {
    if (this.state.busy.value || !this.token) return;
    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await confirmEmail({ ...this.deps, token: this.token });
      if (this.isDisposed) return;
      if (r.ok) this.routing.onConfirmed();
      else this.state.error.value = errorText(r.errors);
    } finally {
      this.state.busy.value = false;
    }
  };
}
