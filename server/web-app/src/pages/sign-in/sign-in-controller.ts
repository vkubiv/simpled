import { EmailNotConfirmed } from "@simpled/backend-client";
import { hasError } from "operation-result.js";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { signIn } from "../../core/ops/auth-ops";
import { errorText } from "../../core/primitives/errors";
import { emailError, hasErrors, requiredError } from "../../core/primitives/forms";
import { SignInActions, type SignInDraft, SignInState } from "./sign-in-model";

export type SignInDeps = Pick<AppDeps, "client" | "session" | "meRepository">;

/** No routing: signing in flips the app's gate, and the route component moves on to `next`. */
export class SignInController extends SubtreeController implements SignInActions {
  readonly state = new SignInState();
  #submitted = false;

  constructor(
    readonly deps: SignInDeps,
    info: string | null = null,
  ) {
    super();
    this.subtree.put(SignInState, this.state);
    this.subtree.put(SignInActions, this);
    this.state.info.value = info;
  }

  setEmail = (value: string) => this.#edit({ email: value });

  setPassword = (value: string) => this.#edit({ password: value });

  submit = async () => {
    if (this.state.busy.value) return;
    this.#submitted = true;
    const draft = this.state.draft.value;
    const errors = validate(draft);
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.busy.value = true;
    this.state.error.value = null;
    this.state.info.value = null;
    this.state.unconfirmedEmail.value = null;
    try {
      const r = await signIn({ ...this.deps, email: draft.email, password: draft.password });
      if (this.isDisposed || r.ok) return;
      if (hasError(r, EmailNotConfirmed)) this.state.unconfirmedEmail.value = draft.email.trim();
      else this.state.error.value = errorText(r.errors);
    } finally {
      this.state.busy.value = false;
    }
  };

  #edit(patch: Partial<SignInDraft>): void {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    if (this.#submitted) this.state.fieldErrors.value = validate(this.state.draft.value);
  }
}

function validate(draft: SignInDraft) {
  return { email: emailError(draft.email), password: requiredError(draft.password, "your password") };
}
