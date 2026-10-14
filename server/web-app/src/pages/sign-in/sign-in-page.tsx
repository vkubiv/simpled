import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Field, invalid, Notice } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { SignInActions, SignInState } from "./sign-in-model";

export function SignInPage() {
  const state = useSubtree(SignInState);
  const actions = useSubtree(SignInActions);
  const observer = useObserver();
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const error = observer.watch(state.error);
  const info = observer.watch(state.info);
  const unconfirmed = observer.watch(state.unconfirmedEmail);
  const busy = observer.watch(state.busy);

  return (
    <AuthCard title="Sign in" subtitle="Welcome back to simpled.">
      <form
        noValidate
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void actions.submit();
        }}
      >
        {info && <Notice kind="ok">{info}</Notice>}
        {error && <Notice kind="danger">{error}</Notice>}
        {unconfirmed && (
          <Notice kind="info">
            Confirm your email first.{" "}
            <Link to={paths.checkEmail(unconfirmed)} className="link">
              Send a new link
            </Link>
          </Notice>
        )}

        <Field id="email" label="Email" error={errors.email}>
          <input
            {...invalid("email", errors.email)}
            type="email"
            autoComplete="email"
            className="field"
            value={draft.email}
            onChange={(e) => actions.setEmail(e.target.value)}
          />
        </Field>

        <Field id="password" label="Password" error={errors.password}>
          <input
            {...invalid("password", errors.password)}
            type="password"
            autoComplete="current-password"
            className="field"
            value={draft.password}
            onChange={(e) => actions.setPassword(e.target.value)}
          />
        </Field>

        <div className="flex justify-end">
          <Link to={paths.forgotPassword} className="link text-sm">
            Forgot password?
          </Link>
        </div>

        <button type="submit" disabled={busy} className="btn btn-primary w-full">
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>

      <p className="mt-6 text-center text-sm text-muted">
        New to simpled?{" "}
        <Link to={paths.register} className="link">
          Create an account
        </Link>
      </p>
    </AuthCard>
  );
}
