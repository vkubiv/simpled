import type { ApiContext } from "@simpled/backend-client";
import type { AuthHandler } from "trunk.js";
import type { AccountSession } from "./core/auth/account-session";
import { createAuthHandler } from "./core/auth/auth-handler";
import type { BackendApi } from "./core/ops/api";
import { MeRepository } from "./core/repositories/me-repository";
import { OrgRepository } from "./core/repositories/org-repository";

/**
 * Everything the app is made of, constructed once at startup. Pages declare the subset they need with
 * `Pick<AppDeps, ...>` and receive it through `pick()`.
 */
export interface AppDeps {
  readonly client: BackendApi;
  /** Ops that handle a 401 themselves (restoring, signing in) read it directly; the rest go through `authHandler`. */
  readonly session: AccountSession;
  readonly authHandler: AuthHandler<ApiContext>;
  readonly meRepository: MeRepository;
  readonly orgRepository: OrgRepository;
}

export function createAppDeps(o: { client: BackendApi; session: AccountSession }): AppDeps {
  return {
    client: o.client,
    session: o.session,
    authHandler: createAuthHandler(o.session),
    meRepository: new MeRepository(),
    orgRepository: new OrgRepository(),
  };
}
