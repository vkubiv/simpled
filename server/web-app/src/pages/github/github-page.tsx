import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, Field, invalid, LoadFailed, Notice, PageHeader, Section, Spinner } from "../../components/ui";
import { GitHubActions, GitHubState } from "./github-model";

const TOKEN_HINT =
  "A fine-grained personal access token with read access to Contents and Metadata of the repositories.";

export function GitHubPage() {
  const state = useSubtree(GitHubState);
  const actions = useSubtree(GitHubActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const connections = observer.watch(state.connections);
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const create = observer.watch(state.create);
  const editing = observer.watch(state.editing);
  const editErrors = observer.watch(state.editErrors);
  const save = observer.watch(state.save);
  const busy = observer.watch(state.busy);
  const error = observer.watch(state.error);
  if (!org) return null;

  const canManage = org.permissions.canManageGitHubConnections;
  return (
    <>
      <PageHeader
        title="GitHub"
        subtitle="Tokens simpled reads your repositories with, to find their environments and deployments."
      />

      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {load.kind === "loading" && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-6">
          {error && <Notice kind="danger">{error}</Notice>}

          <Section title="Connections" description="Projects link repositories through one of these.">
            {connections.length === 0 ? (
              <EmptyState>No GitHub connection yet.</EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {connections.map((c) =>
                  editing?.connectionId === c.id ? (
                    <li key={c.id} className="py-3">
                      <form
                        noValidate
                        className="space-y-3"
                        onSubmit={(e) => {
                          e.preventDefault();
                          void actions.saveConnection();
                        }}
                      >
                        {save.error && <Notice kind="danger">{save.error}</Notice>}
                        <div className="grid gap-3 sm:grid-cols-2">
                          <Field id={`name-${c.id}`} label="Name" error={editErrors.name}>
                            <input
                              {...invalid(`name-${c.id}`, editErrors.name)}
                              className="field"
                              value={editing.name}
                              onChange={(e) => actions.editConnection({ name: e.target.value })}
                            />
                          </Field>
                          <Field id={`token-${c.id}`} label="New token" error={editErrors.token} hint="Empty keeps it.">
                            <input
                              {...invalid(`token-${c.id}`, editErrors.token)}
                              type="password"
                              autoComplete="off"
                              className="field font-mono"
                              value={editing.token}
                              onChange={(e) => actions.editConnection({ token: e.target.value })}
                            />
                          </Field>
                        </div>
                        <div className="flex justify-end gap-2">
                          <button type="button" className="btn btn-ghost" onClick={actions.cancelEditing}>
                            Cancel
                          </button>
                          <button type="submit" className="btn btn-primary" disabled={save.busy}>
                            {save.busy ? "Checking…" : "Save"}
                          </button>
                        </div>
                      </form>
                    </li>
                  ) : (
                    <li key={c.id} className="flex flex-wrap items-center justify-between gap-2 py-2.5">
                      <div className="min-w-0">
                        <p className="text-sm font-medium">{c.name}</p>
                        <p className="text-sm text-muted">
                          as <span className="font-mono">{c.gitHubLogin}</span> ·{" "}
                          {c.repositoryCount === 1 ? "1 repository" : `${c.repositoryCount} repositories`}
                        </p>
                      </div>
                      {canManage && (
                        <div className="flex gap-1">
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            onClick={() => actions.startEditing(c.id)}
                          >
                            Edit
                          </button>
                          <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={busy || c.repositoryCount > 0}
                            title={c.repositoryCount > 0 ? "Unlink its repositories first." : undefined}
                            onClick={() => {
                              if (window.confirm(`Remove the connection ${c.name}?`))
                                void actions.deleteConnection(c.id);
                            }}
                          >
                            Remove
                          </button>
                        </div>
                      )}
                    </li>
                  ),
                )}
              </ul>
            )}
          </Section>

          {canManage && (
            <Section
              title="Add a connection"
              description="simpled checks the token with GitHub and keeps it encrypted."
            >
              <form
                noValidate
                className="space-y-4"
                onSubmit={(e) => {
                  e.preventDefault();
                  void actions.addConnection();
                }}
              >
                {create.error && <Notice kind="danger">{create.error}</Notice>}
                {create.done && <Notice kind="ok">{create.done}</Notice>}
                <div className="grid gap-4 sm:grid-cols-2">
                  <Field id="connection-name" label="Name" error={errors.name}>
                    <input
                      {...invalid("connection-name", errors.name)}
                      className="field"
                      placeholder="Deploy bot"
                      value={draft.name}
                      onChange={(e) => actions.edit({ name: e.target.value })}
                    />
                  </Field>
                  <Field id="connection-token" label="Token" error={errors.token} hint={TOKEN_HINT}>
                    <input
                      {...invalid("connection-token", errors.token)}
                      type="password"
                      autoComplete="off"
                      className="field font-mono"
                      placeholder="github_pat_…"
                      value={draft.token}
                      onChange={(e) => actions.edit({ token: e.target.value })}
                    />
                  </Field>
                </div>
                <div className="flex justify-end">
                  <button type="submit" className="btn btn-primary" disabled={create.busy}>
                    {create.busy ? "Checking…" : "Add connection"}
                  </button>
                </div>
              </form>
            </Section>
          )}
        </div>
      )}
    </>
  );
}
