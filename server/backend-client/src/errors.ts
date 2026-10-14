export abstract class ApiError extends Error {
  constructor(
    message: string,
    readonly originalError?: unknown,
  ) {
    super(message);
    this.name = new.target.name;
  }
}

// ---- transport errors (produced by Transport) ----

/** 401: no session, an expired or revoked one, or (on login) wrong credentials. */
export class ApiNotAuthorized extends ApiError {
  constructor(readonly requestUrl: string) {
    super(`Not authorized: ${requestUrl}`);
  }
}

/** 429 from the rate-limited auth and invitation endpoints: wait a minute and retry. */
export class RateLimited extends ApiError {
  constructor(readonly requestUrl: string) {
    super("Too many attempts. Wait a minute and try again.");
  }
}

/** One entry of the backend's 400 body `[{code, incorrectValue, details}]`. */
export class ValidationError extends ApiError {
  constructor(
    readonly code: string,
    /** The field or route parameter the backend named, e.g. `email` or `orgId`. */
    readonly incorrectValue: string,
    /** The backend's sentence. */
    readonly details: string,
    readonly requestUrl: string,
  ) {
    super(`${code}${incorrectValue ? ` (${incorrectValue})` : ""}: ${details}\n${requestUrl}`);
  }

  // biome-ignore lint/suspicious/noExplicitAny: raw JSON
  static fromJson(json: any, requestUrl: string): ValidationError {
    return new ValidationError(
      String(json?.code ?? ""),
      String(json?.incorrectValue ?? ""),
      String(json?.details ?? ""),
      requestUrl,
    );
  }
}

/** The body did not match the model's schema: the contract drifted, or the client is behind the backend. */
export class ParsingError extends ApiError {}

/**
 * Every failure mapper names the members it transforms or rejects (`throw new UnexpectedApiError(e)`) and
 * returns the rest, so adding a member here breaks the build in every method until it takes a position.
 */
export type TransportError = ApiNotAuthorized | RateLimited | ValidationError;

/** The backend refused a call in a way this method never expects: a bug on one side or the other. */
export class UnexpectedApiError extends ApiError {
  constructor(readonly error: TransportError) {
    super(`Unexpected ${error.name}: ${error.message}`, error);
  }
}

/** Codes the backend's `ValidationResult` emits. */
export const ErrorCode = {
  ValueRequired: "value-required",
  IncorrectValue: "incorrect-value",
  IncorrectState: "incorrect-state",
  AlreadyExist: "already-exist",
  DoesntExist: "doesnt-exist",
  Forbidden: "forbidden",
  EmailNotConfirmed: "email-not-confirmed",
} as const;

// ---- domain errors (produced by API groups) ----

/** A form field the backend refused; `message` is its sentence, `fieldName` the input to attach it to. */
export class InvalidFormField extends ApiError {
  constructor(
    readonly fieldName: string,
    readonly details: string,
    originalError?: unknown,
  ) {
    super(details, originalError);
  }
}

export class InvalidCredentials extends ApiError {
  constructor(originalError?: unknown) {
    super("Wrong email or password.", originalError);
  }
}

export class EmailNotConfirmed extends ApiError {
  constructor(originalError?: unknown) {
    super("The email address is not confirmed yet.", originalError);
  }
}

export class AccountAlreadyExists extends ApiError {
  constructor(originalError?: unknown) {
    super("An account with this email already exists.", originalError);
  }
}

/** An emailed link that is unknown, already used, replaced by a newer one, or expired. */
export class InvalidLink extends ApiError {
  constructor(originalError?: unknown) {
    super("The link is invalid or has expired.", originalError);
  }
}

/** The organization does not exist or the caller is not a member; the backend does not say which. */
export class OrganizationNotFound extends ApiError {
  constructor(
    readonly orgId: number,
    originalError?: unknown,
  ) {
    super(`Organization ${orgId} not found`, originalError);
  }
}

export class TeamNotFound extends ApiError {
  constructor(
    readonly teamId: number,
    originalError?: unknown,
  ) {
    super(`Team ${teamId} not found`, originalError);
  }
}

export class ProjectNotFound extends ApiError {
  constructor(
    readonly projectId: number,
    originalError?: unknown,
  ) {
    super(`Project ${projectId} not found`, originalError);
  }
}

/** Not a member of the organization (or, for team calls, of the team). */
export class MemberNotFound extends ApiError {
  constructor(
    readonly accountId: string,
    originalError?: unknown,
  ) {
    super(`Member ${accountId} not found`, originalError);
  }
}

export class InvitationNotFound extends ApiError {
  constructor(
    readonly invitationId: number,
    originalError?: unknown,
  ) {
    super(`Invitation ${invitationId} not found`, originalError);
  }
}

/** The caller's role in the organization does not allow this. `message` is the sentence to show. */
export class RoleForbidden extends ApiError {
  constructor(
    readonly reason: string,
    /** What was refused: `role` for the action itself, `teamIds` for a manager's project teams. */
    readonly fieldName: string,
    originalError?: unknown,
  ) {
    super(reason, originalError);
  }
}

/** The change would leave the organization without an owner. */
export class LastOwner extends ApiError {
  constructor(originalError?: unknown) {
    super("An organization needs at least one owner.", originalError);
  }
}

/** Already a member of the organization, already in the team, or a team already linked. */
export class AlreadyMember extends ApiError {
  constructor(
    readonly reason: string,
    originalError?: unknown,
  ) {
    super(reason, originalError);
  }
}

/** The signed-in account's email is not the invited one. `message` names the invited email. */
export class InvitationForAnotherEmail extends ApiError {
  constructor(
    readonly reason: string,
    originalError?: unknown,
  ) {
    super(reason, originalError);
  }
}
