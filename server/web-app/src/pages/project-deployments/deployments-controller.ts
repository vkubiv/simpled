import type { Repository } from "@simpled/backend-client";
import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../../app-deps";
import {
  linkRepository,
  loadConnections,
  loadDeploymentEvents,
  loadRepositories,
  syncRepository,
  unlinkRepository,
} from "../../core/ops/monitoring-ops";
import { loadProject } from "../../core/ops/project-ops";
import { errorText, fieldErrors, formLevelErrors } from "../../core/primitives/errors";
import { hasErrors, requiredError } from "../../core/primitives/forms";
import { busyForm, failedForm, failedLoad, idleForm, loaded, loading } from "../../core/primitives/load-status";
import { DeploymentsActions, DeploymentsState, type LinkDraft } from "./deployments-model";

export type DeploymentsDeps = Pick<AppDeps, "client" | "authHandler" | "orgRepository">;

/** monitor-svc checks every minute in production; reading every 15 s shows a change soon after. */
export const REFRESH_MS = 15_000;

/**
 * A project's linked repositories, with the environments and deployments of their envspecs and what the monitor
 * last saw of each. Re-reads the statuses on its own while the page is open.
 */
export class DeploymentsController extends SubtreeController implements DeploymentsActions {
  readonly state = new DeploymentsState();

  constructor(
    readonly deps: DeploymentsDeps,
    readonly orgId: number,
    readonly projectId: number,
    now: () => Date = () => new Date(),
  ) {
    super();
    this.#now = now;
    this.subtree.put(DeploymentsState, this.state);
    this.subtree.put(DeploymentsActions, this);
    this.sync(() => {
      this.state.org.value = deps.orgRepository.org;
    }, [deps.orgRepository]);
    const timer = setInterval(() => void this.refresh(), REFRESH_MS);
    this.autoDispose(() => clearInterval(timer));
    void this.reload();
  }

  readonly #now: () => Date;

  reload = async () => {
    this.state.load.value = loading;
    const [project, repositories] = await Promise.all([
      loadProject({ ...this.deps, projectId: this.projectId }),
      loadRepositories({ ...this.deps, projectId: this.projectId }),
    ]);
    if (this.isDisposed) return;
    if (!project.ok || !repositories.ok) {
      this.state.load.value = failedLoad([
        ...(project.ok ? [] : project.errors),
        ...(repositories.ok ? [] : repositories.errors),
      ]);
      return;
    }
    this.state.canManage.value = project.value.canManage;
    this.#show(repositories.value);
    this.state.load.value = loaded;

    if (project.value.canManage) {
      const connections = await loadConnections({ ...this.deps, orgId: this.orgId });
      if (this.isDisposed || !connections.ok) return;
      this.state.connections.value = connections.value;
      if (connections.value.length === 1) {
        this.state.draft.update((d) => ({ ...d, connectionId: String(connections.value[0].id) }));
      }
    }
  };

  /** Quietly: a failed refresh keeps what is shown, the next one tries again. */
  refresh = async () => {
    if (this.state.load.value.kind !== "loaded" || this.state.busy.value) return;
    const r = await loadRepositories({ ...this.deps, projectId: this.projectId });
    if (this.isDisposed || !r.ok) return;
    this.#show(r.value);
  };

  editLink = (patch: Partial<LinkDraft>) => {
    this.state.draft.update((d) => ({ ...d, ...patch }));
    this.state.fieldErrors.value = {};
    this.state.link.value = idleForm;
  };

  linkRepository = async () => {
    if (this.state.link.value.busy) return;
    const draft = this.state.draft.value;
    const errors = {
      connectionId: draft.connectionId ? undefined : "Pick a GitHub connection.",
      fullName: requiredError(draft.fullName, "the repository"),
    };
    this.state.fieldErrors.value = errors;
    if (hasErrors(errors)) return;

    this.state.link.value = busyForm;
    const r = await linkRepository({
      ...this.deps,
      projectId: this.projectId,
      link: { connectionId: Number(draft.connectionId), fullName: draft.fullName },
    });
    if (this.isDisposed) return;
    if (!r.ok) {
      this.state.fieldErrors.value = fieldErrors(r.errors);
      const rest = formLevelErrors(r.errors);
      this.state.link.value = rest.length > 0 ? failedForm(rest) : idleForm;
      return;
    }
    this.#put(r.value);
    this.state.draft.update((d) => ({ ...d, fullName: "" }));
    this.state.link.value = idleForm;
  };

  syncRepository = (repositoryId: number) =>
    this.#act(async () => {
      const r = await syncRepository({ ...this.deps, repositoryId });
      if (r.ok && !this.isDisposed) this.#put(r.value);
      return r;
    });

  unlinkRepository = (repositoryId: number) =>
    this.#act(async () => {
      const r = await unlinkRepository({ ...this.deps, repositoryId });
      if (r.ok && !this.isDisposed) {
        this.state.repositories.value = this.state.repositories.value.filter((x) => x.id !== repositoryId);
      }
      return r;
    });

  toggleEvents = async (deploymentId: number) => {
    if (this.state.openEvents.value?.deploymentId === deploymentId) {
      this.state.openEvents.value = null;
      return;
    }
    this.state.openEvents.value = { deploymentId, load: loading, events: [] };
    const r = await loadDeploymentEvents({ ...this.deps, deploymentId });
    if (this.isDisposed || this.state.openEvents.value?.deploymentId !== deploymentId) return;
    this.state.openEvents.value = r.ok
      ? { deploymentId, load: loaded, events: r.value }
      : { deploymentId, load: failedLoad(r.errors), events: [] };
  };

  #show(repositories: readonly Repository[]): void {
    this.state.repositories.value = [...repositories];
    this.state.refreshedAt.value = this.#now();
  }

  #put(repository: Repository): void {
    const others = this.state.repositories.value.filter((r) => r.id !== repository.id);
    this.#show([...others, repository].sort((a, b) => `${a.owner}/${a.name}`.localeCompare(`${b.owner}/${b.name}`)));
  }

  async #act(fn: () => Promise<{ ok: boolean; errors?: readonly Error[] }>): Promise<void> {
    if (this.state.busy.value) return;
    this.state.busy.value = true;
    this.state.error.value = null;
    try {
      const r = await fn();
      if (!this.isDisposed && !r.ok) this.state.error.value = errorText(r.errors ?? []);
    } finally {
      if (!this.isDisposed) this.state.busy.value = false;
    }
  }
}
