import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { signOut } from "../../core/ops/auth-ops";
import { acceptInvitation, previewInvitation, registerByInvitation } from "../../core/ops/invitation-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { hasErrors, newPasswordError, requiredError } from "../../core/primitives/forms";
import { busyForm, failedForm, failedLoad, idleForm, loaded, loading } from "../../core/primitives/load-status";
import { InvitationActions, type InvitationDraft, type InvitationMode, InvitationState } from "./invitation-model";

export type InvitationDeps = Pick<AppDeps, "client" | "session" | "authHandler" | "meRepository">;

export interface InvitationRouting {
  /** Joined: the organization's id when known (accepted), or home (registered, where it is the only one). */
  onJoined(orgId: number | null): void;
}

/** The page behind the mailed `invitation?token=` link; works signed in or out. */
export class InvitationController extends SubtreeController implements InvitationActions {
  readonly state = new InvitationState();

  constructor(
    readonly deps: InvitationDeps,
    readonly routing: InvitationRouting,
    readonly token: string,
  ) {
    super();
    this.subtree.put(InvitationState, this.state);
    this.subtree.put(InvitationActions, this);

    this.sync(() => {
      const signedIn = deps.session.stage === "signedIn";
      this.state.signedInEmail.value = signedIn ? (deps.meRepository.me?.account.email ?? null) : null;
    }, [deps.session, deps.meRepository]);

    this.sync(() => {
      this.state.mode.value = modeOf(
        this.state.preview.value?.email,
        this.state.preview.value?.accountExists,
        this.state.signedInEmail.value,
      );
    }, [this.state.preview, this.state.signedInEmail]);

    void this.#load();
  }

  edit = (patch: Partial<InvitationDraft>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    this.state.fieldErrors.update((e) => ({
      ...e,
      ...Object.fromEntries(Object.keys(patch).map((k) => [k, undefined])),
    }));
  };

  accept = async () => {
    if (this.state.form.value.busy) return;
    this.state.form.value = busyForm;
    const r = await acceptInvitation({ ...this.deps, token: this.token });
    if (this.isDisposed) return;
    if (r.ok) {
      this.state.form.value = idleForm;
      this.routing.onJoined(r.value.id);
      return;
    }
    this.state.form.value = failedForm(r.errors);
  };

  register = async () => {
    if (this.state.form.value.busy) return;
    const draft = this.state.draft.value;
    const errors = {
      name: requiredError(draft.name, "your name"),
      password: newPasswordError(draft.password, this.state.preview.value?.email),
    };
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.form.value = busyForm;
    const r = await registerByInvitation({
      ...this.deps,
      token: this.token,
      name: draft.name,
      password: draft.password,
    });
    if (this.isDisposed) return;
    if (r.ok) {
      this.state.form.value = idleForm;
      this.routing.onJoined(null);
      return;
    }
    this.state.fieldErrors.value = fieldErrors(r.errors);
    const rest = formLevelErrors(r.errors);
    this.state.form.value = rest.length > 0 ? failedForm(errorText(rest)) : idleForm;
  };

  /** Signed in as someone else: sign out here, then sign in (or register) as the invited email. */
  signOut = async () => {
    await signOut(this.deps);
  };

  async #load(): Promise<void> {
    if (!this.token) {
      this.state.load.value = { kind: "error", message: "The link is incomplete. Open it again from the email." };
      return;
    }
    this.state.load.value = loading;
    const r = await previewInvitation({ ...this.deps, token: this.token });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.load.value = failedLoad(r.errors);
      return;
    }
    this.state.preview.value = r.value;
    this.state.load.value = loaded;
  }
}

function modeOf(
  invitedEmail: string | undefined,
  accountExists: boolean | undefined,
  signedInEmail: string | null,
): InvitationMode | null {
  if (invitedEmail === undefined) return null;
  if (signedInEmail !== null) return signedInEmail === invitedEmail ? "accept" : "wrongAccount";
  return accountExists ? "signIn" : "register";
}
