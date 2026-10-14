import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, Field, invalid, LoadFailed, Notice, PageHeader, Section, Spinner } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { TeamsActions, TeamsState } from "./teams-model";

export function TeamsPage() {
  const state = useSubtree(TeamsState);
  const actions = useSubtree(TeamsActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const teams = observer.watch(state.teams);
  const newName = observer.watch(state.newName);
  const newNameError = observer.watch(state.newNameError) ?? undefined;
  const create = observer.watch(state.create);
  if (!org) return null;

  return (
    <>
      <PageHeader title="Teams" subtitle="Teams own projects; their members work on them." />

      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {load.kind === "loading" && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-6">
          {org.permissions.canCreateTeam && (
            <Section title="New team">
              <form
                noValidate
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  void actions.createTeam();
                }}
              >
                {create.error && <Notice kind="danger">{create.error}</Notice>}
                <div className="grid gap-3 sm:grid-cols-[1fr_auto] sm:items-start">
                  <Field
                    id="team-name"
                    label="Name"
                    error={newNameError}
                    hint={org.role === "manager" ? "You'll be added to the team." : undefined}
                  >
                    <input
                      {...invalid("team-name", newNameError)}
                      className="field"
                      placeholder="Platform"
                      value={newName}
                      onChange={(e) => actions.setNewName(e.target.value)}
                    />
                  </Field>
                  <button type="submit" className="btn btn-primary sm:mt-[1.625rem]" disabled={create.busy}>
                    {create.busy ? "Creating…" : "Create team"}
                  </button>
                </div>
              </form>
            </Section>
          )}

          <section className="panel">
            {teams.length === 0 ? (
              <EmptyState>No teams yet.</EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {teams.map((t) => (
                  <li key={t.id}>
                    <Link
                      to={paths.team(org.id, t.id)}
                      className="flex items-center gap-3 px-5 py-3.5 transition-colors hover:bg-sunken"
                    >
                      <span className="min-w-0 flex-1 truncate text-sm font-medium">{t.name}</span>
                      {t.isMember && <span className="badge bg-accent-soft text-accent-strong">Your team</span>}
                      <span className="text-sm text-faint">
                        {t.memberCount} {t.memberCount === 1 ? "member" : "members"} · {t.projectCount}{" "}
                        {t.projectCount === 1 ? "project" : "projects"}
                      </span>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        </div>
      )}
    </>
  );
}
