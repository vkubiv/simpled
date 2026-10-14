import { Rx, SubtreeController } from "subtree.js/core";
import type { AppDeps } from "./app-deps";
import { restoreSession, signOut } from "./core/ops/auth-ops";
import { paths } from "./core/primitives/routes";

/** `failed` is main-svc not answering `/Me` at all: a retry, not the sign-in form. */
export type Gate = "loading" | "failed" | "signedOut" | "signedIn";

export class AppState {
  readonly gate = new Rx<Gate>("loading");
  readonly failure = new Rx<string | null>(null);
  /** Where `/` goes: the first organization, or creating one for an account with none. */
  readonly home = new Rx<string>(paths.newOrg);
}

export abstract class AppActions {
  abstract signOut(): Promise<void>;
  abstract retry(): void;
}

export type AppControllerDeps = Pick<AppDeps, "client" | "session" | "meRepository" | "orgRepository">;

/** The app root: asks who is signed in on start, forgets everything on sign-out, and mirrors the gate for the router. */
export class AppController extends SubtreeController implements AppActions {
  readonly state = new AppState();
  #stage: string | null = null;

  constructor(readonly deps: AppControllerDeps) {
    super();
    this.subtree.put(AppState, this.state);
    this.subtree.put(AppActions, this);
    this.sync(() => this.#reconcile(), [deps.session, deps.meRepository]);
  }

  #reconcile(): void {
    const { session, meRepository, orgRepository } = this.deps;
    const me = meRepository.status;
    if (session.stage === "restoring" && me.kind === "idle") void restoreSession(this.deps);
    // Only on the change to signed out: a sign-in in progress writes the account while the stage is still signed out.
    if (session.stage === "signedOut" && this.#stage !== "signedOut") {
      meRepository.reset();
      orgRepository.reset();
    }
    this.#stage = session.stage;
    this.state.failure.value = me.kind === "failed" ? me.message : null;
    const first = meRepository.me?.organizations[0];
    this.state.home.value = first ? paths.org(first.id) : paths.newOrg;
    this.state.gate.value =
      session.stage === "signedIn"
        ? "signedIn"
        : session.stage === "signedOut"
          ? "signedOut"
          : me.kind === "failed"
            ? "failed"
            : "loading";
  }

  signOut = async () => {
    await signOut(this.deps);
  };

  retry = () => {
    this.deps.meRepository.reset();
  };
}
