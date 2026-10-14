import type { Deployment, DeploymentEvent, DeploymentStatus, Repository } from "@simpled/backend-client";
import { Link } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { EmptyState, Field, invalid, LoadFailed, Notice, Section, Spinner } from "../../components/ui";
import { paths } from "../../core/primitives/routes";
import { timeAgo } from "../../core/primitives/time";
import { DeploymentsActions, DeploymentsState, type OpenEvents } from "./deployments-model";

const STATUS: Record<DeploymentStatus, { label: string; cls: string }> = {
  up: { label: "Up", cls: "bg-ok-soft text-ok" },
  down: { label: "Down", cls: "bg-danger-soft text-danger" },
  mismatch: { label: "Mismatch", cls: "bg-warn-soft text-warn" },
  unknown: { label: "Not checked", cls: "bg-sunken text-muted" },
};

const EVENT_LABEL: Record<DeploymentEvent["kind"], string> = {
  "version-changed": "New version",
  down: "Went down",
  up: "Back up",
  mismatch: "Answered for another deployment",
};

/** The project's deployments, by repository and environment. Rendered by the project page. */
export function DeploymentsSection() {
  const state = useSubtree(DeploymentsState);
  const actions = useSubtree(DeploymentsActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const repositories = observer.watch(state.repositories);
  const canManage = observer.watch(state.canManage);
  const busy = observer.watch(state.busy);
  const error = observer.watch(state.error);
  const openEvents = observer.watch(state.openEvents);
  const refreshedAt = observer.watch(state.refreshedAt);
  if (!org) return null;

  const now = new Date();
  return (
    <Section
      title="Deployments"
      description="From the envspecs of linked GitHub repositories; simpled checks each one's version endpoint."
    >
      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {(load.kind === "loading" || load.kind === "idle") && (
        <div className="flex justify-center p-6">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-5">
          {error && <Notice kind="danger">{error}</Notice>}
          {repositories.length === 0 && <EmptyState>No repository is linked yet.</EmptyState>}
          {repositories.map((r) => (
            <RepositoryBlock
              key={r.id}
              repository={r}
              busy={busy}
              now={now}
              openEvents={openEvents}
              onSync={() => void actions.syncRepository(r.id)}
              onUnlink={() => {
                if (window.confirm(`Unlink ${r.owner}/${r.name}? Its deployments' history goes with it.`)) {
                  void actions.unlinkRepository(r.id);
                }
              }}
              onToggleEvents={(id) => void actions.toggleEvents(id)}
            />
          ))}
          {refreshedAt && repositories.length > 0 && (
            <p className="text-xs text-faint">Statuses refresh on their own · read {timeAgo(refreshedAt, now)}</p>
          )}
          {canManage && <LinkForm canAddConnection={org.permissions.canManageGitHubConnections} orgId={org.id} />}
        </div>
      )}
    </Section>
  );
}

function RepositoryBlock({
  repository: r,
  busy,
  now,
  openEvents,
  onSync,
  onUnlink,
  onToggleEvents,
}: {
  repository: Repository;
  busy: boolean;
  now: Date;
  openEvents: OpenEvents | null;
  onSync: () => void;
  onUnlink: () => void;
  onToggleEvents: (deploymentId: number) => void;
}) {
  return (
    <div className="rounded-btn border border-line">
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-sunken px-4 py-2.5">
        <div className="min-w-0 text-sm">
          <a
            href={`https://github.com/${r.owner}/${r.name}`}
            target="_blank"
            rel="noreferrer"
            className="link font-mono"
          >
            {r.owner}/{r.name}
          </a>
          <span className="text-muted">
            {" "}
            · {r.defaultBranch}
            {r.syncedAt && ` · read ${timeAgo(r.syncedAt, now)}`}
          </span>
        </div>
        {r.canManage && (
          <div className="flex gap-1">
            <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={onSync}>
              Sync now
            </button>
            <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={onUnlink}>
              Unlink
            </button>
          </div>
        )}
      </div>
      <div className="space-y-4 p-4">
        {r.syncError && (
          <Notice kind="danger">
            <span className="whitespace-pre-line">{r.syncError}</span>
          </Notice>
        )}
        {r.environments.length === 0 && !r.syncError && (
          <p className="text-sm text-muted">No envspec.yaml on {r.defaultBranch}.</p>
        )}
        {r.environments.map((e) => (
          <div key={e.id}>
            <p className="mb-1.5 text-sm">
              <span className="font-semibold">{e.name}</span>{" "}
              <span className="font-mono text-xs text-faint">{e.path}</span>
            </p>
            <ul className="divide-y divide-line">
              {e.deployments.map((d) => (
                <DeploymentRow
                  key={d.id}
                  deployment={d}
                  now={now}
                  events={openEvents?.deploymentId === d.id ? openEvents : null}
                  onToggleEvents={() => onToggleEvents(d.id)}
                />
              ))}
            </ul>
          </div>
        ))}
      </div>
    </div>
  );
}

function DeploymentRow({
  deployment: d,
  now,
  events,
  onToggleEvents,
}: {
  deployment: Deployment;
  now: Date;
  events: OpenEvents | null;
  onToggleEvents: () => void;
}) {
  const status = STATUS[d.status];
  const behind = d.liveVersion && d.repoVersion && d.liveVersion !== d.repoVersion;
  return (
    <li className="py-2.5">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
        <span className="font-medium">{d.name}</span>
        <span className={`badge ${status.cls}`} title={d.checkError ?? undefined}>
          {status.label}
        </span>
        <span className="text-muted">
          {d.applicationName} <span className="font-mono">{d.liveVersion ?? "–"}</span>
          {behind && (
            <span className="text-warn">
              {" "}
              · <span className="font-mono">{d.repoVersion}</span> in the repository
            </span>
          )}
        </span>
        <span className="ml-auto flex items-center gap-3 text-xs text-faint">
          {d.url ? (
            <a href={d.url} target="_blank" rel="noreferrer" className="hover:underline">
              {d.url.replace(/^https?:\/\//, "")}
            </a>
          ) : (
            <span>no address the monitor can reach</span>
          )}
          {d.checkedAt && <span>checked {timeAgo(d.checkedAt, now)}</span>}
          <button type="button" className="link text-xs" onClick={onToggleEvents}>
            {events ? "Hide history" : "History"}
          </button>
        </span>
      </div>
      {d.checkError && d.status !== "up" && <p className="mt-1 text-xs text-danger">{d.checkError}</p>}
      {events && <EventList events={events} now={now} />}
    </li>
  );
}

function EventList({ events, now }: { events: OpenEvents; now: Date }) {
  if (events.load.kind === "loading") return <Spinner className="mt-2" />;
  if (events.load.kind === "error") return <p className="mt-2 text-xs text-danger">{events.load.message}</p>;
  if (events.events.length === 0) return <p className="mt-2 text-xs text-faint">Nothing has happened yet.</p>;
  return (
    <ol className="mt-2 space-y-1 border-l border-line pl-3 text-xs">
      {events.events.map((e) => (
        <li key={e.id}>
          <span className="text-faint">{timeAgo(e.at, now)}</span> · {EVENT_LABEL[e.kind]}
          {e.version && (
            <>
              {" "}
              <span className="font-mono">
                {e.previousVersion ? `${e.previousVersion} → ` : ""}
                {e.version}
              </span>
            </>
          )}
          {e.details && <span className="text-muted"> · {e.details}</span>}
        </li>
      ))}
    </ol>
  );
}

function LinkForm({ canAddConnection, orgId }: { canAddConnection: boolean; orgId: number }) {
  const state = useSubtree(DeploymentsState);
  const actions = useSubtree(DeploymentsActions);
  const observer = useObserver();
  const connections = observer.watch(state.connections);
  const draft = observer.watch(state.draft);
  const errors = observer.watch(state.fieldErrors);
  const link = observer.watch(state.link);

  if (connections.length === 0) {
    return (
      <p className="text-sm text-muted">
        Linking a repository needs a GitHub connection.{" "}
        {canAddConnection ? (
          <Link to={paths.github(orgId)} className="link">
            Add one
          </Link>
        ) : (
          "Ask an owner or admin to add one."
        )}
      </p>
    );
  }
  return (
    <form
      noValidate
      className="space-y-3 border-t border-line pt-4"
      onSubmit={(e) => {
        e.preventDefault();
        void actions.linkRepository();
      }}
    >
      {link.error && <Notice kind="danger">{link.error}</Notice>}
      <div className="grid gap-3 sm:grid-cols-[1fr_2fr]">
        <Field id="link-connection" label="Connection" error={errors.connectionId}>
          <select
            {...invalid("link-connection", errors.connectionId)}
            className="field"
            value={draft.connectionId}
            onChange={(e) => actions.editLink({ connectionId: e.target.value })}
          >
            <option value="">Pick one…</option>
            {connections.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name} ({c.gitHubLogin})
              </option>
            ))}
          </select>
        </Field>
        <Field id="link-repository" label="Repository" error={errors.fullName} hint="owner/name or its GitHub URL">
          <input
            {...invalid("link-repository", errors.fullName)}
            className="field font-mono"
            placeholder="acme/shop-deployment"
            value={draft.fullName}
            onChange={(e) => actions.editLink({ fullName: e.target.value })}
          />
        </Field>
      </div>
      <div className="flex justify-end">
        <button type="submit" className="btn btn-primary" disabled={link.busy}>
          {link.busy ? "Reading the repository…" : "Link repository"}
        </button>
      </div>
    </form>
  );
}
