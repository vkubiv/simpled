import type { ApiContext } from "../context";
import {
  AccountAlreadyExists,
  ApiNotAuthorized,
  EmailNotConfirmed,
  ErrorCode,
  InvalidCredentials,
  type InvalidFormField,
  InvalidLink,
  type RateLimited,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { parse } from "../model/common";
import { type Registration, SessionToken } from "../model/me";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asFormField, rateLimited } from "./common";

/**
 * `/Auth`: accounts and sessions. Sign-ins without `WithToken` open a cookie session: the browser keeps it, a Node
 * caller keeps it in `ctx.cookieJar`.
 */
export class AuthApi {
  constructor(private readonly t: Transport) {}

  /** Creates an unconfirmed account and its first organization, and mails a confirmation link. */
  async register(
    ctx: ApiContext,
    registration: Registration,
  ): Promise<Result<void, AccountAlreadyExists | InvalidFormField | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/Register", { body: registration });
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError) {
          if (e.code === ErrorCode.AlreadyExist && e.incorrectValue === "email") return new AccountAlreadyExists(e);
          return asFormField(e);
        }
        return rateLimited(e);
      },
    );
  }

  /** Confirms the email behind the mailed token and signs in. */
  async confirmEmail(ctx: ApiContext, token: string): Promise<Result<void, InvalidLink | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/ConfirmEmail", { body: { token } });
    return forward(
      r,
      () => undefined,
      (e) => (e instanceof ValidationError && e.incorrectValue === "token" ? new InvalidLink(e) : rateLimited(e)),
    );
  }

  /** Mails a fresh link to an unconfirmed account; succeeds whether or not one exists. */
  async resendConfirmation(ctx: ApiContext, email: string): Promise<Result<void, RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/ResendConfirmation", { body: { email } });
    return forward(r, () => undefined, rateLimited);
  }

  async login(
    ctx: ApiContext,
    email: string,
    password: string,
  ): Promise<Result<void, InvalidCredentials | EmailNotConfirmed | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/Login", { body: { email, password } });
    return forward(r, () => undefined, loginFailure);
  }

  /** For the CLI: the session token comes back in the body; use it with `withToken`. */
  async loginWithToken(
    ctx: ApiContext,
    email: string,
    password: string,
  ): Promise<Result<SessionToken, InvalidCredentials | EmailNotConfirmed | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/Login", { query: { useToken: true }, body: { email, password } });
    return forward(r, (j) => parse(SessionToken, j), loginFailure);
  }

  /** Revokes the session and drops the cookie. */
  async logout(ctx: ApiContext): Promise<Result<void, ApiNotAuthorized | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/Logout");
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError) throw new UnexpectedApiError(e);
        return e;
      },
    );
  }

  /** Mails a reset link; succeeds whether or not the account exists. */
  async requestPasswordReset(ctx: ApiContext, email: string): Promise<Result<void, RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/RequestPasswordReset", { body: { email } });
    return forward(r, () => undefined, rateLimited);
  }

  /** Sets the new password, confirms the email and signs out every session. Sign in afterwards. */
  async resetPassword(
    ctx: ApiContext,
    token: string,
    newPassword: string,
  ): Promise<Result<void, InvalidLink | InvalidFormField | RateLimited>> {
    const r = await this.t.post(ctx, "/Auth/ResetPassword", { body: { token, newPassword } });
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError) return e.incorrectValue === "token" ? new InvalidLink(e) : asFormField(e);
        return rateLimited(e);
      },
    );
  }
}

/** A 401 from Login means wrong credentials (or a locked account), not a missing session. */
function loginFailure(e: ApiNotAuthorized | RateLimited | ValidationError) {
  if (e instanceof ApiNotAuthorized) return new InvalidCredentials(e);
  if (e instanceof ValidationError) {
    if (e.code === ErrorCode.EmailNotConfirmed) return new EmailNotConfirmed(e);
    throw new UnexpectedApiError(e);
  }
  return e;
}
