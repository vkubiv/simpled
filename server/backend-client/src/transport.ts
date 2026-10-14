import type { ApiContext } from "./context";
import { ApiNotAuthorized, RateLimited, type TransportError, ValidationError } from "./errors";
import { fail, ok, type Result } from "./result";

export interface TransportConfig {
  /** Origin plus prefix: `/api` in the browser (same origin), `http://localhost:7080/api` in Node. */
  baseUrl: string;
  printLogs?: boolean;
  fetch?: typeof fetch;
}

export type Query = Record<string, string | number | boolean | undefined>;

export interface RequestOptions {
  body?: unknown;
  query?: Query;
}

const RELATIVE_ORIGIN = "http://relative.invalid";

export class Transport {
  private readonly fetchImpl: typeof fetch;

  constructor(private readonly cfg: TransportConfig) {
    // Bound, never `cfg.fetch ?? fetch`: called as `this.fetchImpl(...)`, the browser's fetch would get the
    // Transport as its receiver and throw "Illegal invocation"; Node's does not check, so e2e would not notice.
    this.fetchImpl = cfg.fetch ?? globalThis.fetch.bind(globalThis);
  }

  get(ctx: ApiContext, path: string, o?: RequestOptions) {
    return this.request(ctx, "GET", path, o);
  }
  post(ctx: ApiContext, path: string, o?: RequestOptions) {
    return this.request(ctx, "POST", path, o);
  }
  put(ctx: ApiContext, path: string, o?: RequestOptions) {
    return this.request(ctx, "PUT", path, o);
  }
  delete(ctx: ApiContext, path: string, o?: RequestOptions) {
    return this.request(ctx, "DELETE", path, o);
  }

  async request(
    ctx: ApiContext,
    method: string,
    path: string,
    o: RequestOptions = {},
  ): Promise<Result<unknown, TransportError>> {
    const url = this.buildUrl(path, o.query);

    const headers: Record<string, string> = { Accept: "application/json" };
    if (o.body !== undefined) headers["Content-Type"] = "application/json";
    if (ctx.authToken) headers.Authorization = `Bearer ${ctx.authToken}`;
    const cookie = ctx.cookieJar?.cookieHeader();
    if (cookie) headers.Cookie = cookie;

    // A failed fetch (offline, DNS, refused) throws: rare, and never a business outcome.
    const res = await this.fetchImpl(url, {
      method,
      headers,
      body: o.body === undefined ? undefined : JSON.stringify(o.body),
      credentials: "same-origin",
    });

    ctx.cookieJar?.storeSetCookie(res.headers.getSetCookie());

    const text = await res.text();
    const data = text.length ? safeJson(text) : undefined;

    if (this.cfg.printLogs) {
      console.log(`${method} ${url} -> ${res.status}\n${text}`);
    }

    if (res.status === 401) return fail(new ApiNotAuthorized(url));
    if (res.status === 429) return fail(new RateLimited(url));
    if (res.status === 400 && Array.isArray(data)) {
      return fail(...data.map((e) => ValidationError.fromJson(e, url)));
    }
    if (res.status < 300) return ok(data);

    throw new Error(
      `Unexpected response ${res.status} for ${method} ${url}\n` +
        `Request body: ${JSON.stringify(o.body)}\nResponse body: ${text}`,
    );
  }

  private buildUrl(path: string, query?: Query): string {
    const base = this.cfg.baseUrl.replace(/\/?$/, "/");
    const rel = path.replace(/^\//, "");
    const absolute = /^[a-z][a-z0-9+.-]*:\/\//i.test(base);
    const url = absolute ? new URL(rel, base) : new URL(base + rel, RELATIVE_ORIGIN);
    for (const [k, v] of Object.entries(query ?? {})) {
      if (v !== undefined) url.searchParams.set(k, String(v));
    }
    return absolute ? url.toString() : url.pathname + url.search;
  }
}

function safeJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}
