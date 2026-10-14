import { errorText } from "./errors";

/** Where a list or a detail is in its life. */
export type LoadStatus =
  | { readonly kind: "idle" }
  | { readonly kind: "loading" }
  | { readonly kind: "loaded" }
  | { readonly kind: "error"; readonly message: string };

export const idleLoad: LoadStatus = { kind: "idle" };
export const loading: LoadStatus = { kind: "loading" };
export const loaded: LoadStatus = { kind: "loaded" };
export const failedLoad = (errors: readonly Error[]): LoadStatus => ({ kind: "error", message: errorText(errors) });

/** One form's last outcome: in flight, the sentence to show, or what just succeeded. */
export interface FormStatus {
  readonly busy: boolean;
  readonly error: string | null;
  readonly done: string | null;
}

export const idleForm: FormStatus = { busy: false, error: null, done: null };
export const busyForm: FormStatus = { busy: true, error: null, done: null };
export const doneForm = (message: string): FormStatus => ({ busy: false, error: null, done: message });
export const failedForm = (error: string | readonly Error[]): FormStatus => ({
  busy: false,
  error: typeof error === "string" ? error : errorText(error),
  done: null,
});
