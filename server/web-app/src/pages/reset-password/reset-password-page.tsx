import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Field, invalid, Notice } from "../../components/ui";
import { MIN_PASSWORD } from "../../core/primitives/forms";
import { paths } from "../../core/primitives/routes";
import { ResetPasswordActions, ResetPasswordState } from "./reset-password-model";

export function ResetPasswordPage() {
  const state = useSubtree(ResetPasswordState);
  const actions = useSubtree(ResetPasswordActions);
  const observer = useObserver();
  const password = observer.watch(state.password);
  const passwordError = observer.watch(state.passwordError) ?? undefined;
  const error = observer.watch(state.error);
  const busy = observer.watch(state.busy);

  return (
    <AuthCard title="Choose a new password" subtitle="You'll be signed out everywhere and sign in again with it.">
      <form
        noValidate
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void actions.submit();
        }}
      >
        {error && (
          <Notice kind="danger">
            {error}{" "}
            <Link to={paths.forgotPassword} className="link">
              Request a new link
            </Link>
          </Notice>
        )}
        <Field id="password" label="New password" error={passwordError} hint={`At least ${MIN_PASSWORD} characters.`}>
          <input
            {...invalid("password", passwordError)}
            type="password"
            autoComplete="new-password"
            className="field"
            value={password}
            onChange={(e) => actions.setPassword(e.target.value)}
          />
        </Field>
        <button type="submit" className="btn btn-primary w-full" disabled={busy}>
          {busy ? "Saving…" : "Set new password"}
        </button>
      </form>
    </AuthCard>
  );
}
