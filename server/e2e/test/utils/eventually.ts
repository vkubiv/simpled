const WAIT_MS = 20_000;
const EVERY_MS = 250;

/**
 * Calls `read` until `pred` holds for its answer, for what monitor-svc does on its own schedule (every 2 s in the
 * local deployment). The timeout names `what` and the last answer.
 */
export async function eventually<T>(read: () => Promise<T>, pred: (value: T) => boolean, what: string): Promise<T> {
  const deadline = Date.now() + WAIT_MS;
  for (;;) {
    const value = await read();
    if (pred(value)) return value;
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${what}; last: ${JSON.stringify(value)}`);
    await new Promise((resolve) => setTimeout(resolve, EVERY_MS));
  }
}
