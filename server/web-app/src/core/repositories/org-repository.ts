import type { Organization } from "@simpled/backend-client";
import { ChangeNotifier } from "subtree.js/core";

export type OrgStatus =
  | { readonly kind: "idle" }
  | { readonly kind: "loading"; readonly orgId: number }
  | { readonly kind: "loaded"; readonly org: Organization }
  | { readonly kind: "notFound"; readonly orgId: number }
  | { readonly kind: "failed"; readonly orgId: number; readonly message: string };

/**
 * The organization the signed-in area is showing, with the caller's role and permissions. The shell loads it for
 * the `/orgs/:orgId` in the URL; pages read the permissions from here.
 */
export class OrgRepository extends ChangeNotifier {
  #status: OrgStatus = { kind: "idle" };

  get status(): OrgStatus {
    return this.#status;
  }

  get org(): Organization | null {
    return this.#status.kind === "loaded" ? this.#status.org : null;
  }

  set(status: OrgStatus): void {
    this.#status = status;
    this.notifyListeners();
  }

  reset(): void {
    if (this.#status.kind !== "idle") this.set({ kind: "idle" });
  }
}
