/**
 * Who is calling. The web app relies on the browser's HttpOnly `session` cookie (same origin as the API) and
 * passes `browser`. Node callers carry either a `cookieJar` (the e2e suite) or a bearer `authToken` from
 * `auth.loginWithToken` (the simpled CLI).
 */
export interface ApiContext {
  readonly authToken?: string;
  readonly cookieJar?: CookieJar;
}

export interface CookieJar {
  cookieHeader(): string | undefined;
  storeSetCookie(setCookieHeaders: string[]): void;
}

export const anonymous: ApiContext = {};

/** In the browser the cookie store does the work, so the context is empty. */
export const browser: ApiContext = {};

export function withToken(authToken: string): ApiContext {
  return { authToken };
}

/** In-memory jar for one API host: enough for Node callers. An emptied or expired cookie is dropped. */
export class MemoryCookieJar implements CookieJar {
  private readonly cookies = new Map<string, string>();

  cookieHeader(): string | undefined {
    if (this.cookies.size === 0) return undefined;
    return [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; ");
  }

  storeSetCookie(headers: string[]): void {
    for (const header of headers) {
      const [pair, ...attributes] = header.split(";");
      const eq = pair.indexOf("=");
      if (eq <= 0) continue;
      const name = pair.slice(0, eq).trim();
      const value = pair.slice(eq + 1).trim();
      const expired = attributes.some((a) => {
        const [k, v] = a.split("=");
        return k.trim().toLowerCase() === "expires" && v !== undefined && new Date(v.trim()) <= new Date();
      });
      if (value === "" || expired) this.cookies.delete(name);
      else this.cookies.set(name, value);
    }
  }
}
