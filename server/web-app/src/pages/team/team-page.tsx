import { useState } from "react";
import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, LoadFailed, Notice, PageHeader, RoleBadge, Section, Spinner } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { TeamActions, TeamState } from "./team-model";

export function TeamPage() {
  const state = useSubtree(TeamState);
  const actions = useSubtree(TeamActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const team = observer.watch(state.team);
  const candidates = observer.watch(state.candidates);
  const name = observer.watch(state.name);
  const rename = observer.watch(state.rename);
  const busy = observer.watch(state.busy);
  const error = observer.watch(state.error);
  const [toAdd, setToAdd] = useState("");
  if (!org) return null;

  if (load.kind === "error") return <LoadFailed message={load.message} onRetry={() => void actions.reload()} />;
  if (load.kind !== "loaded" || !team) {
    return (
      <div className="flex justify-center p-10">
        <Spinner />
      </div>
    );
  }

  return (
    <>
      <p className="mb-2 text-sm">
        <Link to={paths.teams(org.id)} className="link">
          ← Teams
        </Link>
      </p>
      <PageHeader
        title={team.name}
        subtitle={team.isMember ? "You're in this team." : undefined}
        actions={
          team.canManage && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={busy}
              onClick={() => {
                if (window.confirm(`Delete the team ${team.name}? Its projects stay.`)) void actions.deleteTeam();
              }}
            >
              Delete team
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
        <Section title={`Members (${team.members.length})`}>
          {team.canManage && candidates.length > 0 && (
            <form
              className="mb-4 flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                if (toAdd) void actions.addMember(toAdd).then(() => setToAdd(""));
              }}
            >
              <select
                aria-label="Member to add"
                className="field"
                value={toAdd}
                onChange={(e) => setToAdd(e.target.value)}
              >
                <option value="">Add a member of {org.name}…</option>
                {candidates.map((m) => (
                  <option key={m.accountId} value={m.accountId}>
                    {m.name} ({m.email})
                  </option>
                ))}
              </select>
              <button type="submit" className="btn btn-primary" disabled={busy || !toAdd}>
                Add
              </button>
            </form>
          )}
          {team.members.length === 0 ? (
            <EmptyState>Nobody is in this team yet.</EmptyState>
          ) : (
            <ul className="divide-y divide-line">
              {team.members.map((m) => (
                <li key={m.accountId} className="flex items-center gap-3 py-2.5">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{m.name}</p>
                    <p className="truncate text-sm text-muted">{m.email}</p>
                  </div>
                  <RoleBadge role={m.role} />
                  {team.canManage && (
                    <button
                      type="button"
                      className="btn btn-ghost btn-sm"
                      disabled={busy}
                      onClick={() => void actions.removeMember(m.accountId)}
                    >
                      Remove
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
        </Section>

        <Section title={`Projects (${team.projects.length})`} description="Link teams from a project's page.">
          {team.projects.length === 0 ? (
            <EmptyState>No projects are linked to this team.</EmptyState>
          ) : (
            <ul className="divide-y divide-line">
              {team.projects.map((p) => (
                <li key={p.id} className="flex items-center justify-between py-2.5">
                  <Link to={paths.project(org.id, p.id)} className="link">
                    {p.name}
                  </Link>
                  <span className="font-mono text-xs text-faint">{p.slug}</span>
                </li>
              ))}
            </ul>
          )}
        </Section>

        {team.canManage && (
          <Section title="Rename">
            <form
              className="flex flex-wrap gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                void actions.saveName();
              }}
            >
              <input
                aria-label="Team name"
                className="field max-w-sm"
                value={name}
                onChange={(e) => actions.setName(e.target.value)}
              />
              <button type="submit" className="btn btn-secondary" disabled={rename.busy || name === team.name}>
                {rename.busy ? "Saving…" : "Save"}
              </button>
            </form>
            {rename.error && <p className="field-error">{rename.error}</p>}
            {rename.done && <p className="hint text-ok">{rename.done}</p>}
          </Section>
        )}
      </div>
    </>
  );
}
