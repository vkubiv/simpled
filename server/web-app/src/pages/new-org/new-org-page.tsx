import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { Field, invalid, Notice } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { TopBar } from "../../shell/shell-layout";
import { NewOrgActions, NewOrgState } from "./new-org-model";

export function NewOrgPage() {
  const state = useSubtree(NewOrgState);
  const actions = useSubtree(NewOrgActions);
  const observer = useObserver();
  const name = observer.watch(state.name);
  const nameError = observer.watch(state.nameError) ?? undefined;
  const error = observer.watch(state.error);
  const busy = observer.watch(state.busy);
  const account = observer.watch(state.account);
  const organizations = observer.watch(state.organizations);

  return (
    <div className="min-h-screen">
      <TopBar
        account={account}
        organizations={organizations}
        currentOrgId={null}
        currentName={null}
        onSignOut={() => void actions.signOut()}
      />
      <main className="mx-auto max-w-md px-4 py-12">
        <h1 className="text-xl font-semibold tracking-tight">
          {organizations.length === 0 ? "Create your organization" : "New organization"}
        </h1>
        <p className="mt-1 text-sm text-muted">
          {organizations.length === 0
            ? "You're not in any organization yet. Create one, or ask a teammate to invite you."
            : "You'll be its owner. Invite others once it exists."}
        </p>
        <form
          noValidate
          className="panel mt-6 space-y-4 p-6"
          onSubmit={(e) => {
            e.preventDefault();
            void actions.submit();
          }}
        >
          {error && <Notice kind="danger">{error}</Notice>}
          <Field id="name" label="Name" error={nameError}>
            <input
              {...invalid("name", nameError)}
              autoComplete="organization"
              className="field"
              value={name}
              onChange={(e) => actions.setName(e.target.value)}
            />
          </Field>
          <div className="flex justify-end gap-2">
            {organizations.length > 0 && (
              <Link to={paths.home} className="btn btn-secondary">
                Cancel
              </Link>
            )}
            <button type="submit" className="btn btn-primary" disabled={busy}>
              {busy ? "Creating…" : "Create organization"}
            </button>
          </div>
        </form>
      </main>
    </div>
  );
}
