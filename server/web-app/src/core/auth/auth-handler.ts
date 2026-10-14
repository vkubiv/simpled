import type { ApiContext } from "@simpled/backend-client";
import { type AuthContextProvider, AuthHandler } from "trunk.js";

/**
 * There is no re-authentication flow: a 401 means the session ended, `sessionFetch` turns the app signed out, and
 * the gate shows the sign-in page. A failed call goes straight back to its caller.
 */
export function createAuthHandler(provider: AuthContextProvider<ApiContext>): AuthHandler<ApiContext> {
  return new AuthHandler({ provider, onAuthFailure: (call) => call.cancel() });
}
