import { useObserver, useSubtree } from "subtree.js/react";
import { Field, Notice, PageHeader, Section } from "../../components/ui";
import { OrgSettingsActions, OrgSettingsState } from "./org-settings-model";

export function OrgSettingsPage() {
  const state = useSubtree(OrgSettingsState);
  const actions = useSubtree(OrgSettingsActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const name = observer.watch(state.name);
  const rename = observer.watch(state.rename);
  const confirmation = observer.watch(state.deleteConfirmation);
  const canDelete = observer.watch(state.canDelete);
  const danger = observer.watch(state.danger);
  if (!org) return null;

  const owner = org.permissions.canManageOrganization;
  return (
    <>
      <PageHeader title="Settings" />
      <div className="space-y-6">
        <Section title="Organization" description="The slug stays the same when you rename.">
          <form
            noValidate
            className="space-y-4"
            onSubmit={(e) => {
              e.preventDefault();
              void actions.saveName();
            }}
          >
            {rename.error && <Notice kind="danger">{rename.error}</Notice>}
            {rename.done && <Notice kind="ok">{rename.done}</Notice>}
            <div className="grid gap-4 sm:grid-cols-2">
              <Field id="org-name" label="Name">
                <input
                  id="org-name"
                  className="field"
                  value={name}
                  readOnly={!owner}
                  onChange={(e) => actions.setName(e.target.value)}
                />
              </Field>
              <Field id="org-slug" label="Slug">
                <input id="org-slug" className="field bg-sunken font-mono text-muted" value={org.slug} readOnly />
              </Field>
            </div>
            {owner && (
              <div className="flex justify-end">
                <button type="submit" className="btn btn-secondary" disabled={rename.busy || name === org.name}>
                  {rename.busy ? "Saving…" : "Save"}
                </button>
              </div>
            )}
          </form>
        </Section>

        <section className="panel border-danger/30">
          <header className="border-b border-line px-5 py-4">
            <h2 className="text-sm font-semibold text-danger">Danger zone</h2>
          </header>
          <div className="space-y-6 p-5">
            {danger.error && <Notice kind="danger">{danger.error}</Notice>}

            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <p className="text-sm font-medium">Leave {org.name}</p>
                <p className="text-sm text-muted">You lose access until someone invites you again.</p>
              </div>
              <button
                type="button"
                className="btn btn-danger"
                disabled={danger.busy}
                onClick={() => {
                  if (window.confirm(`Leave ${org.name}?`)) void actions.leave();
                }}
              >
                Leave organization
              </button>
            </div>

            {owner && (
              <div className="border-t border-line pt-6">
                <p className="text-sm font-medium">Delete {org.name}</p>
                <p className="text-sm text-muted">
                  Removes its members, invitations, teams and projects. This can't be undone.
                </p>
                <form
                  className="mt-3 flex flex-wrap gap-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void actions.deleteOrg();
                  }}
                >
                  <input
                    aria-label="Type the organization's name to confirm"
                    className="field max-w-xs"
                    placeholder={org.name}
                    value={confirmation}
                    onChange={(e) => actions.setDeleteConfirmation(e.target.value)}
                  />
                  <button type="submit" className="btn btn-danger" disabled={!canDelete || danger.busy}>
                    Delete organization
                  </button>
                </form>
              </div>
            )}
          </div>
        </section>
      </div>
    </>
  );
}
