export type Result<T, E> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly errors: readonly E[] };

export function ok<T>(value: T): Result<T, never> {
  return { ok: true, value };
}

export function fail<E>(...errors: E[]): Result<never, E> {
  return { ok: false, errors };
}

// biome-ignore lint/suspicious/noExplicitAny: matches any constructor signature
type Ctor<E> = abstract new (...args: any[]) => E;

/** True if the result failed with at least one error of the given class. */
export function hasError<E>(r: Result<unknown, unknown>, cls: Ctor<E>): boolean {
  return !r.ok && r.errors.some((e) => e instanceof cls);
}

/** First error of the given class, or undefined. */
export function getError<E>(r: Result<unknown, unknown>, cls: Ctor<E>): E | undefined {
  if (r.ok) return undefined;
  return r.errors.find((e): e is E => e instanceof cls);
}

/** Value of a successful result. Throws if it failed: use only after checking errors. */
export function unwrap<T>(r: Result<T, unknown>): T {
  if (!r.ok) throw new Error(`Unhandled expected errors:\n${describeErrors(r.errors)}`);
  return r.value;
}

/**
 * Transform a result. `success` maps the value, `failure` maps each error.
 * A `failure` mapper that meets an error it did not expect should throw.
 */
export function forward<T, U, E1, E2>(
  r: Result<T, E1>,
  success: (value: T) => U,
  failure: (error: E1) => E2,
): Result<U, E2> {
  if (r.ok) return ok(success(r.value));
  return fail(...r.errors.map(failure));
}

export function describeErrors(errors: readonly unknown[]): string {
  return errors.map((e) => (e instanceof Error ? `${e.name}: ${e.message}` : String(e))).join("\n");
}
