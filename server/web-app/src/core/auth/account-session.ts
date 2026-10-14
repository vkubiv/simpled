import type { ApiContext } from "@simpled/backend-client";
import { ChangeNotifier } from "subtree.js/core";
import type { AuthContextProvider } from "trunk.js";

export type SessionStage = "restoring" | "signedOut" | "signedIn";

/**
 * Whether this app holds a main-svc session. The session itself is an HttpOnly cookie the app cannot read, so the
 * stage is what the last answer said: `/Me` on start, a sign-in, a sign-out, or any 401 (`sessionFetch`).
 * `context` is what every call carries: `browser` in the browser, a cookie jar in Node (the e2e suite).
 */
export class AccountSession extends ChangeNotifier implements AuthContextProvider<ApiContext> {
  #stage: SessionStage = "restoring";

  constructor(readonly context: ApiContext) {
    super();
  }

  get stage(): SessionStage {
    return this.#stage;
  }

  async authorizeCall(): Promise<ApiContext> {
    return this.context;
  }

  setStage(stage: SessionStage): void {
    if (stage === this.#stage) return;
    this.#stage = stage;
    this.notifyListeners();
  }
}

/**
 * `fetch` for the client: a 401 on any call means the session is gone (expired, revoked, signed out elsewhere),
 * so a signed-in app turns signed out and the gate sends the person to sign in. A 401 while signed out (a wrong
 * password) changes nothing.
 */
export function sessionFetch(session: AccountSession, base: typeof fetch = globalThis.fetch.bind(globalThis)) {
  return async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const res = await base(input, init);
    if (res.status === 401 && session.stage === "signedIn") session.setStage("signedOut");
    return res;
  };
}
