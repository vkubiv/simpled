import { MainClient, MemoryCookieJar } from "@simpled/backend-client";
import { type AppDeps, createAppDeps } from "simpled-web-app/src/app-deps";
import { AccountSession, sessionFetch } from "simpled-web-app/src/core/auth/account-session";
import { signIn } from "simpled-web-app/src/core/ops/auth-ops";
import type { Disposable } from "subtree.js/core";
import { env } from "./env";
import { expectSuccess } from "./expect";
import { PASSWORD } from "./sugar";

export { settled, waitFor } from "./rx";

/**
 * The web app, run in this process against the stack: the real `createAppDeps` over a real `MainClient`, with a
 * cookie jar standing in for the browser's cookie store. Tests build page controllers on `deps` and drive them as
 * the unit tests do. One instance is one browser.
 */
export class WebApp implements Disposable {
  readonly session = new AccountSession({ cookieJar: new MemoryCookieJar() });
  readonly deps: AppDeps = createAppDeps({
    client: new MainClient({ baseUrl: env.apiUrl, printLogs: env.printLogs, fetch: sessionFetch(this.session) }),
    session: this.session,
  });
  readonly #owned: Disposable[] = [];

  /** A controller this app disposes with itself. */
  own<T extends Disposable>(controller: T): T {
    this.#owned.push(controller);
    return controller;
  }

  dispose(): void {
    for (const c of this.#owned.splice(0).reverse()) c.dispose();
  }
}

/** The app signed in as `email`, through the same op the sign-in page uses. */
export async function signedInApp(email: string, password = PASSWORD): Promise<WebApp> {
  const app = new WebApp();
  expectSuccess(await signIn({ ...app.deps, email, password }));
  return app;
}
