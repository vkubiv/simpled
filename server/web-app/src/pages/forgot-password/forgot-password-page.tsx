import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Field, invalid, Notice } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { ForgotPasswordActions, ForgotPasswordState } from "./forgot-password-model";

export function ForgotPasswordPage() {
  const state = useSubtree(ForgotPasswordState);
  const actions = useSubtree(ForgotPasswordActions);
  const observer = useObserver();
  const email = observer.watch(state.email);
  const emailError = observer.watch(state.emailError) ?? undefined;
  const status = observer.watch(state.status);

  return (
    <AuthCard title="Reset your password" subtitle="We'll email you a link to choose a new one.">
      <form
        noValidate
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void actions.submit();
        }}
      >
        {status.done && <Notice kind="ok">{status.done}</Notice>}
        {status.error && <Notice kind="danger">{status.error}</Notice>}
        <Field id="email" label="Email" error={emailError}>
          <input
            {...invalid("email", emailError)}
            type="email"
            autoComplete="email"
            className="field"
            value={email}
            onChange={(e) => actions.setEmail(e.target.value)}
          />
        </Field>
        <button type="submit" className="btn btn-primary w-full" disabled={status.busy}>
          {status.busy ? "Sending…" : "Send reset link"}
        </button>
        <p className="text-center text-sm">
          <Link to={paths.signIn()} className="link">
            Back to sign in
          </Link>
        </p>
      </form>
    </AuthCard>
  );
}
