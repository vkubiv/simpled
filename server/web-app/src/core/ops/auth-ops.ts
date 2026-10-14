import {
  type AccountAlreadyExists,
  ApiNotAuthorized,
  type EmailNotConfirmed,
  type InvalidCredentials,
  type InvalidFormField,
  type InvalidLink,
  type RateLimited,
  type Registration,
} from "@simpled/backend-client";
import { type AsyncResult, ok } from "operation-result.js";
import type { AccountSession } from "../auth/account-session";
import { thrownText } from "../primitives/errors";
import type { MeRepository } from "../repositories/me-repository";
import type { BackendApi } from "./api";

type SessionDeps = { client: BackendApi; session: AccountSession; meRepository: MeRepository };

/**
 * Asks main-svc who is signed in: the account and its organizations land in `meRepository` and the session turns
 * signed in, or a 401 turns it signed out. A failure to answer at all leaves the stage alone and marks `failed`,
 * which the app shows with a retry. Handles its own 401, so it calls without `authHandler`. Single flight.
 */
export async function restoreSession(o: SessionDeps): Promise<void> {
  if (o.meRepository.status.kind === "loading") return;
  o.meRepository.set({ kind: "loading" });
  try {
    const r = await o.client.me.get(await o.session.authorizeCall());
    if (r.ok) {
      o.meRepository.set({ kind: "loaded", me: r.value });
      o.session.setStage("signedIn");
    } else {
      // Stage first: an idle account while still restoring would read as "not asked yet" and ask again.
      o.session.setStage("signedOut");
      o.meRepository.reset();
    }
  } catch (e) {
    o.meRepository.set({ kind: "failed", message: thrownText(e, "simpled is not answering.") });
  }
}

/** Reloads the account's organizations after one is created, renamed, joined, left or deleted. */
export async function refreshMe(o: SessionDeps): Promise<void> {
  const r = await o.client.me.get(await o.session.authorizeCall());
  if (r.ok) o.meRepository.set({ kind: "loaded", me: r.value });
}

export async function signIn(
  o: SessionDeps & { email: string; password: string },
): AsyncResult<void, InvalidCredentials | EmailNotConfirmed | RateLimited> {
  const r = await o.client.auth.login(o.session.context, o.email.trim(), o.password);
  if (!r.ok) return r;
  await restoreSession(o);
  return ok();
}

/** Creates the unconfirmed account with its first organization; main-svc mails the confirmation link. */
export function register(o: {
  client: BackendApi;
  session: AccountSession;
  registration: Registration;
}): AsyncResult<void, AccountAlreadyExists | InvalidFormField | RateLimited> {
  const { email, name, organizationName, password } = o.registration;
  return o.client.auth.register(o.session.context, {
    email: email.trim(),
    name: name.trim(),
    organizationName: organizationName.trim(),
    password,
  });
}

/** Confirms the email behind the link, which also signs in. */
export async function confirmEmail(o: SessionDeps & { token: string }): AsyncResult<void, InvalidLink | RateLimited> {
  const r = await o.client.auth.confirmEmail(o.session.context, o.token);
  if (!r.ok) return r;
  await restoreSession(o);
  return ok();
}

export function resendConfirmation(o: {
  client: BackendApi;
  session: AccountSession;
  email: string;
}): AsyncResult<void, RateLimited> {
  return o.client.auth.resendConfirmation(o.session.context, o.email.trim());
}

export function requestPasswordReset(o: {
  client: BackendApi;
  session: AccountSession;
  email: string;
}): AsyncResult<void, RateLimited> {
  return o.client.auth.requestPasswordReset(o.session.context, o.email.trim());
}

/** Sets the new password; main-svc signs out every session, so the person signs in again afterwards. */
export function resetPassword(o: {
  client: BackendApi;
  session: AccountSession;
  token: string;
  newPassword: string;
}): AsyncResult<void, InvalidLink | InvalidFormField | RateLimited> {
  return o.client.auth.resetPassword(o.session.context, o.token, o.newPassword);
}

/** Ends the session here even when main-svc already considers it gone. */
export async function signOut(o: { client: BackendApi; session: AccountSession }): Promise<void> {
  const r = await o.client.auth.logout(o.session.context);
  if (!r.ok && !r.errors.every((e) => e instanceof ApiNotAuthorized)) {
    console.warn("logout failed", r.errors);
  }
  o.session.setStage("signedOut");
}
