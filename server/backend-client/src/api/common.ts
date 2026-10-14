import {
  ApiNotAuthorized,
  ErrorCode,
  InvalidFormField,
  RateLimited,
  RoleForbidden,
  type TransportError,
  UnexpectedApiError,
  type ValidationError,
} from "../errors";

/** Tail of a signed-in call's failure mapper: 401 passes through, anything else is a bug. */
export function signedIn(e: TransportError): ApiNotAuthorized {
  if (e instanceof ApiNotAuthorized) return e;
  throw new UnexpectedApiError(e);
}

/** Tail of an anonymous, rate-limited call's failure mapper. */
export function rateLimited(e: TransportError): RateLimited {
  if (e instanceof RateLimited) return e;
  throw new UnexpectedApiError(e);
}

export function asForbidden(e: ValidationError): RoleForbidden | undefined {
  return e.code === ErrorCode.Forbidden ? new RoleForbidden(e.details, e.incorrectValue, e) : undefined;
}

export function asFormField(e: ValidationError): InvalidFormField {
  return new InvalidFormField(e.incorrectValue, e.details, e);
}
