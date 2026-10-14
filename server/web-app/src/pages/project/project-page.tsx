import { useState } from "react";
import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, Field, invalid, LoadFailed, Notice, PageHeader, Section, Spinner } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { ProjectActions, ProjectState } from "./project-model";

export function ProjectPage() {
  const state = useSubtree(ProjectState);
  const actions = useSubtree(ProjectActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const project = observer.watch(state.project);
  const linkable = observer.watch(state.linkable);
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const save = observer.watch(state.save);
  const busy = observer.watch(state.busy);
  const error = observer.watch(state.error);
  const [toLink, setToLink] = useState("");
  if (!org) return null;

  if (load.kind === "error") return <LoadFailed message={load.message} onRetry={() => void actions.reload()} />;
  if (load.kind !== "loaded" || !project) {
    return (
      <div className="flex justify-center p-10">
        <Spinner />
      </div>
    );
  }

  return (
    <>
      <p className="mb-2 text-sm">
        <Link to={paths.projects(org.id)} className="link">
          ← Projects
        </Link>
      </p>
      <PageHeader
        title={project.name}
        subtitle={<span className="font-mono text-xs">{project.slug}</span>}
        actions={
          project.canManage && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={busy}
              onClick={() => {
                if (window.confirm(`Delete the project ${project.name}?`)) void actions.deleteProject();
              }}
            >
              Delete project
            </button>
          )
        }
      />
      {error && (
        <div className="mb-4">
          <Notice kind="danger">{error}</Notice>
        </div>
      )}

      <div className="space-y-6">
        <Section title="Teams" description="Members of these teams work on the project.">
          {project.canManage && linkable.length > 0 && (
            <form
              className="mb-4 flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (toLink) void actions.linkTeam(Number(toLink)).then(() => setToLink(""));
              }}
            >
              <select
                aria-label="Team to link"
                className="field"
                value={toLink}
                onChange={(e) => setToLink(e.target.value)}
              >
                <option value="">Link a team…</option>
                {linkable.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.name}
                  </option>
                ))}
              </select>
              <button type="submit" className="btn btn-primary" disabled={busy || !toLink}>
                Link
              </button>
            </form>
          )}
          {project.teams.length === 0 ? (
            <EmptyState>No team is linked.</EmptyState>
          ) : (
            <ul className="divide-y divide-line">
              {project.teams.map((t) => (
                <li key={t.id} className="flex items-center justify-between py-2.5">
                  <Link to={paths.team(org.id, t.id)} className="link">
                    {t.name}
                  </Link>
                  {project.canManage && (
                    <button
                      type="button"
                      className="btn btn-ghost btn-sm"
                      disabled={busy}
                      onClick={() => void actions.unlinkTeam(t.id)}
                    >
                      Unlink
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Section>

        {project.canManage && (
          <Section title="Details">
            <form
              noValidate
              className="space-y-4"
              onSubmit={(e) => {
                e.preventDefault();
                void actions.saveDetails();
              }}
            >
              {save.error && <Notice kind="danger">{save.error}</Notice>}
              {save.done && <Notice kind="ok">{save.done}</Notice>}
              <div className="grid gap-4 sm:grid-cols-2">
                <Field id="name" label="Name" error={errors.name}>
                  <input
                    {...invalid("name", errors.name)}
                    className="field"
                    value={draft.name}
                    onChange={(e) => actions.edit({ name: e.target.value })}
                  />
                </Field>
                <Field id="slug" label="Slug" error={errors.slug} hint="The CLI will address the project by it.">
                  <input
                    {...invalid("slug", errors.slug)}
                    className="field font-mono"
                    value={draft.slug}
                    onChange={(e) => actions.edit({ slug: e.target.value })}
                  />
                </Field>
              </div>
              <div className="flex justify-end">
                <button type="submit" className="btn btn-secondary" disabled={save.busy}>
                  {save.busy ? "Saving…" : "Save"}
                </button>
              </div>
            </form>
          </Section>
        )}
      </div>
    </>
  );
}
