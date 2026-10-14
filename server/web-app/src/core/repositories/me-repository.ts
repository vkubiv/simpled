import type { Me } from "@simpled/backend-client";
import { ChangeNotifier } from "subtree.js/core";

/** `failed` is not signed out: `/Me` did not answer at all (main-svc down), so signing in again would not help. */
export type MeStatus =
  | { readonly kind: "idle" }
  | { readonly kind: "loading" }
  | { readonly kind: "loaded"; readonly me: Me }
  | { readonly kind: "failed"; readonly message: string };

/** The signed-in account and its organizations with roles: the org switcher, and where `/` lands. */
export class MeRepository extends ChangeNotifier {
  #status: MeStatus = { kind: "idle" };

  get status(): MeStatus {
    return this.#status;
  }

  get me(): Me | null {
    return this.#status.kind === "loaded" ? this.#status.me : null;
  }

  set(status: MeStatus): void {
    this.#status = status;
    this.notifyListeners();
  }

  reset(): void {
    if (this.#status.kind !== "idle") this.set({ kind: "idle" });
  }
}
