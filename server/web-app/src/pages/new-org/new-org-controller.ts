import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import { signOut } from "../../core/ops/auth-ops";
import { createOrg } from "../../core/ops/org-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { requiredError } from "../../core/primitives/forms";
import { NewOrgActions, NewOrgState } from "./new-org-model";

export type NewOrgDeps = Pick<AppDeps, "client" | "session" | "authHandler" | "meRepository">;

export interface NewOrgRouting {
  onCreated(orgId: number): void;
}

/** Another organization, owned by the caller; also where an account with none lands. */
export class NewOrgController extends SubtreeController implements NewOrgActions {
  readonly state = new NewOrgState();

  constructor(
    readonly deps: NewOrgDeps,
    readonly routing: NewOrgRouting,
  ) {
    super();
    this.subtree.put(NewOrgState, this.state);
    this.subtree.put(NewOrgActions, this);
    this.sync(() => {
      const me = deps.meRepository.me;
      this.state.account.value = me?.account ?? null;
      this.state.organizations.value = me?.organizations ?? [];
    }, [deps.meRepository]);
  }

  setName = (value: string) => {
    this.state.name.value = value;
    this.state.nameError.value = null;
  };

  submit = async () => {
    if (this.state.busy.value) return;
    const name = this.state.name.value;
    const error = requiredError(name, "a name");
    this.state.nameError.value = error ?? null;
    if (error) return;

    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await createOrg({ ...this.deps, name });
      if (this.isDisposed) return;
      if (r.ok) {
        this.routing.onCreated(r.value.id);
        return;
      }
      this.state.nameError.value = fieldErrors(r.errors).name ?? null;
      const rest = formLevelErrors(r.errors);
      if (rest.length > 0) this.state.error.value = errorText(rest);
    } finally {
      this.state.busy.value = false;
    }
  };

  signOut = async () => {
    await signOut(this.deps);
  };
}
