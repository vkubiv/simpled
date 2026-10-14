import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Notice } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { ConfirmEmailActions, ConfirmEmailState } from "./confirm-email-model";

export function ConfirmEmailPage() {
  const state = useSubtree(ConfirmEmailState);
  const actions = useSubtree(ConfirmEmailActions);
  const observer = useObserver();
  const busy = observer.watch(state.busy);
  const error = observer.watch(state.error);

  return (
    <AuthCard title="Confirm your email" subtitle="One click and your account is ready.">
      <div className="space-y-4">
        {error && <Notice kind="danger">{error} Sign in to get a new link sent.</Notice>}
        <button type="button" className="btn btn-primary w-full" disabled={busy} onClick={() => void actions.confirm()}>
          {busy ? "Confirming…" : "Confirm and sign in"}
        </button>
        <p className="text-center text-sm">
          <Link to={paths.signIn()} className="link">
            Back to sign in
          </Link>
        </p>
      </div>
    </AuthCard>
  );
}
