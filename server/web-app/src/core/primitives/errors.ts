import { InvalidFormField } from "@simpled/backend-client";

/** The sentence to show for a failed result. The client's error classes already write `message` as one. */
export function errorText(errors: readonly Error[]): string {
  return errors[0]?.message ?? "Something went wrong.";
}

/** The sentence for a thrown value, where a transport failure surfaces as an exception. */
export function thrownText(error: unknown, fallback = "Something went wrong."): string {
  return error instanceof Error ? error.message || fallback : fallback;
}

/** Field name to sentence, for the backend's `InvalidFormField`s; other errors are left to `errorText`. */
export function fieldErrors(errors: readonly Error[]): Record<string, string> {
  const map: Record<string, string> = {};
  for (const e of errors) {
    if (e instanceof InvalidFormField && e.fieldName && !(e.fieldName in map)) map[e.fieldName] = e.details;
  }
  return map;
}

/** Errors that are not about one field: shown above the form. */
export function formLevelErrors(errors: readonly Error[]): Error[] {
  return errors.filter((e) => !(e instanceof InvalidFormField && e.fieldName));
}
