import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, LoadFailed, PageHeader, RoleBadge, Section, Spinner } from "../../components/ui";
import { ROLE_DESCRIPTIONS } from "../../core/primitives/roles";
import { paths } from "../../core/primitives/routes";
import { OrgOverviewActions, OrgOverviewState } from "./org-overview-model";

export function OrgOverviewPage() {
  const state = useSubtree(OrgOverviewState);
  const actions = useSubtree(OrgOverviewActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const teams = observer.watch(state.teams);
  const projects = observer.watch(state.projects);
  const memberCount = observer.watch(state.memberCount);
  if (!org) return null;

  const myTeams = teams.filter((t) => t.isMember);
  return (
    <>
      <PageHeader
        title={org.name}
        subtitle={
          <span className="inline-flex items-center gap-2">
            <span className="font-mono text-xs">{org.slug}</span>
            <span aria-hidden="true">·</span>
            You are <RoleBadge role={org.role} />
          </span>
        }
        actions={
          org.permissions.canCreateProject && (
            <Link to={paths.projects(org.id)} className="btn btn-primary">
              New project
            </Link>
          )
        }
      />
      <p className="-mt-3 mb-6 text-sm text-muted">{ROLE_DESCRIPTIONS[org.role]}</p>

      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {load.kind === "loading" && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-6">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <Stat label="Projects" value={projects.length} to={paths.projects(org.id)} />
            <Stat label="Teams" value={teams.length} to={paths.teams(org.id)} />
            <Stat label="Members" value={memberCount} to={paths.members(org.id)} />
          </div>

          <Section title="Your teams" description="Teams you belong to; their projects are yours to work on.">
            {myTeams.length === 0 ? (
              <EmptyState>You're not in a team yet.</EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {myTeams.map((t) => (
                  <li key={t.id} className="flex items-center justify-between py-2.5">
                    <Link to={paths.team(org.id, t.id)} className="link">
                      {t.name}
                    </Link>
                    <span className="text-sm text-faint">
                      {t.memberCount} members · {t.projectCount} projects
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Section>

          <Section title="Projects">
            {projects.length === 0 ? (
              <EmptyState>No projects yet.</EmptyState>
            ) : (
              <ul className="divide-y divide-line">
                {projects.map((p) => (
                  <li key={p.id} className="flex items-center justify-between gap-3 py-2.5">
                    <Link to={paths.project(org.id, p.id)} className="link truncate">
                      {p.name}
                    </Link>
                    <span className="truncate text-sm text-faint">
                      {p.teams.length === 0 ? "No team" : p.teams.map((t) => t.name).join(", ")}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>
      )}
    </>
  );
}

function Stat({ label, value, to }: { label: string; value: number; to: string }) {
  return (
    <Link to={to} className="panel block p-4 transition-colors hover:border-line-strong">
      <p className="text-sm text-muted">{label}</p>
      <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
    </Link>
  );
}
