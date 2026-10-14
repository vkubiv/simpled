/** The client-side rules, mirroring main-svc's (AccountValidation); the backend still has the last word. */

export const MIN_PASSWORD = 10;

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

export type FieldErrors<K extends string> = Partial<Record<K, string>>;

export const hasErrors = (errors: object): boolean => Object.values(errors).some((v) => v !== undefined);

export function emailError(email: string): string | undefined {
  return EMAIL_PATTERN.test(email.trim()) ? undefined : "Enter a valid email address.";
}

export function newPasswordError(password: string, email = ""): string | undefined {
  if (password.length < MIN_PASSWORD) return `Use at least ${MIN_PASSWORD} characters.`;
  if (email && password.trim().toLowerCase() === email.trim().toLowerCase()) return "Don't use your email as password.";
  return undefined;
}

export function requiredError(value: string, what: string): string | undefined {
  return value.trim() === "" ? `Enter ${what}.` : undefined;
}

/** Same rule as main-svc's `Slug`: lower-case letters, digits and single inner dashes. */
export function slugError(slug: string): string | undefined {
  if (slug === "") return undefined;
  return /^[a-z0-9]+(-[a-z0-9]+)*$/.test(slug) && slug.length <= 40
    ? undefined
    : "Use lower-case letters, digits and single dashes.";
}

/** Only same-app paths, so `?next=` cannot send anyone elsewhere. */
export function safeNext(next: string | null): string {
  return next?.startsWith("/") && !next.startsWith("//") ? next : "/";
}
