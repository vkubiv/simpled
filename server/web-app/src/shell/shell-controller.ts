import { SubtreeController } from "subtree.js/core";
import type { AppDeps } from "../app-deps";
import { signOut } from "../core/ops/auth-ops";
import { loadOrg } from "../core/ops/org-ops";
import { ShellActions, ShellState } from "./shell-model";

export type ShellDeps = Pick<AppDeps, "client" | "session" | "authHandler" | "meRepository" | "orgRepository">;

/**
 * The signed-in area of one organization: loads it for `orgId` (rebuilt when the URL's organization changes) and
 * mirrors it, with the account's other organizations for the switcher. Pages read the organization's permissions
 * from `orgRepository` themselves.
 */
export class ShellController extends SubtreeController implements ShellActions {
  readonly state: ShellState;

  constructor(
    readonly deps: ShellDeps,
    readonly orgId: number,
  ) {
    super();
    this.state = new ShellState(orgId);
    this.subtree.put(ShellState, this.state);
    this.subtree.put(ShellActions, this);

    const current = deps.orgRepository.status;
    if (!(current.kind === "loaded" && current.org.id === orgId)) void loadOrg({ ...deps, orgId });

    this.sync(() => {
      const status = deps.orgRepository.status;
      if (status.kind === "loaded" && status.org.id === orgId) {
        this.state.org.value = status.org;
        this.state.view.value = "ready";
      } else if (status.kind === "notFound" && status.orgId === orgId) {
        this.state.view.value = "notFound";
      } else if (status.kind === "failed" && status.orgId === orgId) {
        this.state.failure.value = status.message;
        this.state.view.value = "failed";
      }
    }, [deps.orgRepository]);

    this.sync(() => {
      const me = deps.meRepository.me;
      this.state.account.value = me?.account ?? null;
      this.state.organizations.value = me?.organizations ?? [];
    }, [deps.meRepository]);
  }

  signOut = async () => {
    await signOut(this.deps);
  };

  retry = async () => {
    await loadOrg({ ...this.deps, orgId: this.orgId });
  };
}
