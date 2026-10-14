import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { AuthCard, Field, invalid, Notice, RoleBadge, Spinner } from "../../components/ui";
import { MIN_PASSWORD } from "../../core/primitives/forms";
import { ROLE_DESCRIPTIONS } from "../../core/primitives/roles";
import { paths } from "../../core/primitives/routes";
import { InvitationActions, InvitationState } from "./invitation-model";

export function InvitationPage({ returnTo }: { returnTo: string }) {
  const state = useSubtree(InvitationState);
  const actions = useSubtree(InvitationActions);
  const observer = useObserver();
  const load = observer.watch(state.load);
  const preview = observer.watch(state.preview);
  const mode = observer.watch(state.mode);
  const signedInEmail = observer.watch(state.signedInEmail);
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const form = observer.watch(state.form);

  if (load.kind === "loading" || load.kind === "idle") {
    return (
      <AuthCard title="Invitation">
        <div className="flex justify-center py-6">
          <Spinner />
        </div>
      </AuthCard>
    );
  }

  if (load.kind === "error" || !preview) {
    return (
      <AuthCard title="This invitation can't be used" subtitle={load.kind === "error" ? load.message : undefined}>
        <p className="text-sm text-muted">
          It may have been accepted already, replaced by a newer invitation, revoked, or it expired after 7 days. Ask
          whoever invited you to send a new one.
        </p>
        <Link to={paths.home} className="btn btn-secondary mt-5 w-full">
          Go to simpled
        </Link>
      </AuthCard>
    );
  }

  return (
    <AuthCard
      title={`Join ${preview.organizationName}`}
      subtitle={
        <>
          {preview.invitedBy} invited <span className="font-medium text-ink">{preview.email}</span> to join as{" "}
          <RoleBadge role={preview.role} />
        </>
      }
    >
      <p className="mb-5 text-sm text-muted">{ROLE_DESCRIPTIONS[preview.role]}</p>
      <div className="space-y-4">
        {form.error && <Notice kind="danger">{form.error}</Notice>}

        {mode === "accept" && (
          <button
            type="button"
            className="btn btn-primary w-full"
            disabled={form.busy}
            onClick={() => void actions.accept()}
          >
            {form.busy ? "Joining…" : `Join ${preview.organizationName}`}
          </button>
        )}

        {mode === "wrongAccount" && (
          <>
            <Notice kind="info">
              You're signed in as {signedInEmail}. Sign out, then open this link again as {preview.email}.
            </Notice>
            <button type="button" className="btn btn-secondary w-full" onClick={() => void actions.signOut()}>
              Sign out
            </button>
          </>
        )}

        {mode === "signIn" && (
          <>
            <p className="text-sm">{preview.email} already has a simpled account. Sign in to accept.</p>
            <Link to={paths.signIn(returnTo)} className="btn btn-primary w-full">
              Sign in to accept
            </Link>
          </>
        )}

        {mode === "register" && (
          <form
            noValidate
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void actions.register();
            }}
          >
            <Field id="email" label="Email">
              <input id="email" className="field bg-sunken text-muted" value={preview.email} readOnly />
            </Field>
            <Field id="name" label="Your name" error={errors.name}>
              <input
                {...invalid("name", errors.name)}
                autoComplete="name"
                className="field"
                value={draft.name}
                onChange={(e) => actions.edit({ name: e.target.value })}
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
            <button type="submit" className="btn btn-primary w-full" disabled={form.busy}>
              {form.busy ? "Creating account…" : "Create account and join"}
            </button>
          </form>
        )}
      </div>
    </AuthCard>
  );
}
