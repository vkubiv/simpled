import { randomUUID } from "node:crypto";
import { env } from "./env";

/**
 * simpled-fakes (server/fakes/server.mjs): GitHub's API for main-svc and monitor-svc, and the gateway of every
 * monitored deployment. Names are unique per instance, so tests share one fakes server like they share the
 * database.
 */
export class Fakes {
  private readonly run = randomUUID().slice(0, 8);

  /** A unique GitHub owner, token or host part. */
  unique(base: string): string {
    return `${base}-${this.run}`;
  }

  /** A token GitHub accepts as `login`. */
  async token(token: string, login = "octocat"): Promise<void> {
    await call("PUT", `/tokens/${encodeURIComponent(token)}`, { login });
  }

  /** A repository `tokens` can read, holding `files` on its default branch; called again, it replaces them. */
  async repository(owner: string, name: string, tokens: string[], files: Record<string, string>): Promise<void> {
    await call("PUT", `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(name)}`, { tokens, files });
  }

  /** What `host`'s `/.well-known/simpled/version` answers, as a simpled gateway would. */
  async site(host: string, version: { application: string; deployment: string; version: string }): Promise<void> {
    await call("PUT", `/sites/${encodeURIComponent(host)}`, {
      body: { ...version, deployed_at: Math.floor(Date.now() / 1000) },
    });
  }

  /** `host` stops answering. */
  async siteDown(host: string): Promise<void> {
    await call("DELETE", `/sites/${encodeURIComponent(host)}`);
  }
}

async function call(method: string, path: string, body?: unknown): Promise<void> {
  const r = await fetch(`${env.fakesUrl}/_fakes${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`simpled-fakes ${method} ${path}: ${r.status} ${await r.text()}`);
}

/** An envspec with TLS off, so the monitor asks `http://<host>`, which simpled-fakes answers. */
export function envSpec(host: string, deployments: Record<string, { application: string; version?: string }>) {
  const lines = ["type: docker", "gateway:", "  name: e2e", "  tls:", "    disable: true", "  hosts:"];
  lines.push(`    web: [${host}]`, "deployments:");
  for (const [name, d] of Object.entries(deployments)) {
    lines.push(`  ${name}:`, "    primary_host: web", "    application:", `      name: ${d.application}`);
    if (d.version) lines.push(`      version: "${d.version}"`);
  }
  return `${lines.join("\n")}\n`;
}

export function appSpec(name: string, version: string): string {
  return `name: ${name}\nversion: ${version}\napp_services: {}\n`;
}
