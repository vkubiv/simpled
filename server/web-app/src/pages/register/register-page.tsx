import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Field, invalid, Notice } from "../../components/ui";
import { MIN_PASSWORD } from "../../core/primitives/forms";
import { paths } from "../../core/primitives/routes";
import { RegisterActions, RegisterState } from "./register-model";

export function RegisterPage() {
  const state = useSubtree(RegisterState);
  const actions = useSubtree(RegisterActions);
  const observer = useObserver();
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const error = observer.watch(state.error);
  const busy = observer.watch(state.busy);

  return (
    <AuthCard title="Create your account" subtitle="You'll be the owner of your first organization.">
      <form
        noValidate
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault();
          void actions.submit();
        }}
      >
        {error && <Notice kind="danger">{error}</Notice>}

        <Field id="name" label="Your name" error={errors.name}>
          <input
            {...invalid("name", errors.name)}
            autoComplete="name"
            className="field"
            value={draft.name}
            onChange={(e) => actions.edit({ name: e.target.value })}
          />
        </Field>

        <Field id="email" label="Work email" error={errors.email}>
          <input
            {...invalid("email", errors.email)}
            type="email"
            autoComplete="email"
            className="field"
            value={draft.email}
            onChange={(e) => actions.edit({ email: e.target.value })}
          />
        </Field>

        <Field id="password" label="Password" error={errors.password} hint={`At least ${MIN_PASSWORD} characters.`}>
          <input
            {...invalid("password", errors.password)}
            type="password"
            autoComplete="new-password"
            className="field"
            value={draft.password}
            onChange={(e) => actions.edit({ password: e.target.value })}
          />
        </Field>

        <Field
          id="organizationName"
          label="Organization"
          error={errors.organizationName}
          hint="Your company or team. You can rename it later."
        >
          <input
            {...invalid("organizationName", errors.organizationName)}
            autoComplete="organization"
            className="field"
            value={draft.organizationName}
            onChange={(e) => actions.edit({ organizationName: e.target.value })}
          />
        </Field>

        <button type="submit" disabled={busy} className="btn btn-primary w-full">
          {busy ? "Creating account…" : "Create account"}
        </button>
      </form>

      <p className="mt-6 text-center text-sm text-muted">
        Already have an account?{" "}
        <Link to={paths.signIn()} className="link">
          Sign in
        </Link>
      </p>
    </AuthCard>
  );
}
