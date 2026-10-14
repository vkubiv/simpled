import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, Field, invalid, LoadFailed, Notice, PageHeader, Section, Spinner } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { ProjectsActions, ProjectsState } from "./projects-model";

export function ProjectsPage() {
  const state = useSubtree(ProjectsState);
  const actions = useSubtree(ProjectsActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const projects = observer.watch(state.projects);
  const teamOptions = observer.watch(state.teamOptions);
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const create = observer.watch(state.create);
  if (!org) return null;

  const manager = org.role === "manager";
  return (
    <>
      <PageHeader title="Projects" subtitle="What simpled deploys. A project's teams are the people who work on it." />

      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {load.kind === "loading" && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-6">
          {org.permissions.canCreateProject && (
            <Section title="New project">
              {manager && teamOptions.length === 0 ? (
                <p className="text-sm text-muted">
                  A manager's project belongs to one of their teams.{" "}
                  <Link to={paths.teams(org.id)} className="link">
                    Create a team first
                  </Link>
                  .
                </p>
              ) : (
                <form
                  noValidate
                  className="space-y-4"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void actions.createProject();
                  }}
                >
                  {create.error && <Notice kind="danger">{create.error}</Notice>}
                  <div className="grid gap-4 sm:grid-cols-2">
                    <Field id="project-name" label="Name" error={errors.name}>
                      <input
                        {...invalid("project-name", errors.name)}
                        className="field"
                        placeholder="Web shop"
                        value={draft.name}
                        onChange={(e) => actions.edit({ name: e.target.value })}
                      />
                    </Field>
                    <Field id="project-slug" label="Slug" error={errors.slug} hint="Optional; made from the name.">
                      <input
                        {...invalid("project-slug", errors.slug)}
                        className="field font-mono"
                        placeholder="web-shop"
                        value={draft.slug}
                        onChange={(e) => actions.edit({ slug: e.target.value })}
                      />
                    </Field>
                  </div>
                  {teamOptions.length > 0 && (
                    <fieldset>
                      <legend className="label">Teams{manager ? "" : " (optional)"}</legend>
                      <div className="flex flex-wrap gap-2">
                        {teamOptions.map((t) => {
                          const checked = draft.teamIds.includes(t.id);
                          return (
                            <label
                              key={t.id}
                              className={`flex cursor-pointer items-center gap-2 rounded-btn border px-3 py-1.5 text-sm ${
                                checked ? "border-accent bg-accent-soft text-accent-strong" : "border-line-strong"
                              }`}
                            >
                              <input
                                type="checkbox"
                                className="accent-accent"
                                checked={checked}
                                onChange={() => actions.toggleTeam(t.id)}
                              />
                              {t.name}
                            </label>
                          );
                        })}
                      </div>
                      {errors.teamIds && <p className="field-error">{errors.teamIds}</p>}
                    </fieldset>
                  )}
                  <div className="flex justify-end">
                    <button type="submit" className="btn btn-primary" disabled={create.busy}>
                      {create.busy ? "Creating…" : "Create project"}
                    </button>
                  </div>
                </form>
              )}
            </Section>
          )}

          <section className="panel">
            {projects.length === 0 ? (
              <EmptyState>No projects yet.</EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {projects.map((p) => (
                  <li key={p.id}>
                    <Link
                      to={paths.project(org.id, p.id)}
                      className="flex flex-wrap items-center gap-x-3 gap-y-1 px-5 py-3.5 transition-colors hover:bg-sunken"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium">{p.name}</span>
                        <span className="block font-mono text-xs text-faint">{p.slug}</span>
                      </span>
                      {p.isMember && <span className="badge bg-accent-soft text-accent-strong">Your project</span>}
                      <span className="truncate text-sm text-muted">
                        {p.teams.length === 0 ? "No team" : p.teams.map((t) => t.name).join(", ")}
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
