import type { ValueListenable } from "subtree.js/core";

const WAIT_MS = 15_000;

/**
 * Resolves with the first value that satisfies `pred`, the current one included. Listeners fire synchronously on
 * every change, so nothing is polled; the timeout names the value it was stuck on.
 */
export function waitFor<T>(rx: ValueListenable<T>, pred: (value: T) => boolean, what = "value"): Promise<T> {
  if (pred(rx.value)) return Promise.resolve(rx.value);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      rx.removeListener(listener);
      reject(new Error(`Timed out waiting for ${what}; last: ${JSON.stringify(rx.value)}`));
    }, WAIT_MS);
    const listener = () => {
      if (!pred(rx.value)) return;
      clearTimeout(timer);
      rx.removeListener(listener);
      resolve(rx.value);
    };
    rx.addListener(listener);
  });
}

interface LoadLike {
  readonly kind: "idle" | "loading" | "loaded" | "error";
  readonly message?: string;
}

/** Waits for a page's load to finish, failing with its message if it failed. */
export async function settled(status: ValueListenable<LoadLike>, what = "load"): Promise<void> {
  const s = await waitFor(status, (s) => s.kind === "loaded" || s.kind === "error", what);
  if (s.kind === "error") throw new Error(`${what} failed: ${s.message}`);
}
