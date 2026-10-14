import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Notice } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { CheckEmailActions, CheckEmailState } from "./check-email-model";

export function CheckEmailPage() {
  const state = useSubtree(CheckEmailState);
  const actions = useSubtree(CheckEmailActions);
  const observer = useObserver();
  const resend = observer.watch(state.resend);

  return (
    <AuthCard
      title="Check your email"
      subtitle={
        state.email ? (
          <>
            We sent a confirmation link to <span className="font-medium text-ink">{state.email}</span>. Open it to
            finish creating your account.
          </>
        ) : (
          "We sent you a confirmation link. Open it to finish creating your account."
        )
      }
    >
      <div className="space-y-4">
        {resend.done && <Notice kind="ok">{resend.done}</Notice>}
        {resend.error && <Notice kind="danger">{resend.error}</Notice>}
        <p className="text-sm text-muted">The link is valid for 24 hours. Nothing arrived? Check spam, or:</p>
        {state.email && (
          <button
            type="button"
            className="btn btn-secondary w-full"
            disabled={resend.busy}
            onClick={() => void actions.resend()}
          >
            {resend.busy ? "Sending…" : "Send a new link"}
          </button>
        )}
        <p className="text-center text-sm">
          <Link to={paths.signIn()} className="link">
            Back to sign in
          </Link>
        </p>
      </div>
    </AuthCard>
  );
}
