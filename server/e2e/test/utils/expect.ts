import { describeErrors, getError, type Result } from "@simpled/backend-client";

export function expectSuccess<T>(r: Result<T, unknown>): T {
  if (!r.ok) throw new Error(`Expecting success but got:\n${describeErrors(r.errors)}`);
  return r.value;
}

// biome-ignore lint/suspicious/noExplicitAny: matches any constructor signature
export function expectError<E>(r: Result<unknown, unknown>, cls: abstract new (...a: any[]) => E): E {
  const e = getError(r, cls);
  if (!e) throw new Error(`Expecting ${cls.name} but got: ${r.ok ? "success" : describeErrors(r.errors)}`);
  return e;
}
