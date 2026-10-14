// simpled-fakes: GitHub's REST API and the gateway of every monitored deployment, for local runs and e2e.
// No dependencies; local-infra.yaml mounts this file into node:alpine.
//
// Tests script it through /_fakes (all bodies JSON):
//   PUT    /_fakes/tokens/:token          {login}                         a token GitHub accepts
//   PUT    /_fakes/repos/:owner/:name     {tokens, files, defaultBranch?} a repository those tokens can read
//   DELETE /_fakes/repos/:owner/:name
//   PUT    /_fakes/sites/:host            {status?, body}                 what /.well-known/simpled/version answers
//   DELETE /_fakes/sites/:host                                            the host stops answering (502)
//
// monitor-svc reaches deployments with simpled-fakes as its HTTP proxy (PROBE_PROXY_URL), so a request whose
// target is an absolute URL is a deployment's; everything else is the GitHub API or /_fakes.

import { createHash } from "node:crypto";
import { createServer } from "node:http";

const tokens = new Map(); // token -> login
const repos = new Map(); // "owner/name" lower-cased -> { owner, name, defaultBranch, tokens: Set, files: {path: text} }
const sites = new Map(); // host lower-cased -> { status, body }

const repoKey = (owner, name) => `${owner}/${name}`.toLowerCase();

function send(res, status, body, type = "application/json") {
  const text = typeof body === "string" ? body : JSON.stringify(body);
  res.writeHead(status, { "content-type": type, "cache-control": "no-store" });
  res.end(text);
}

async function readJson(req) {
  let text = "";
  for await (const chunk of req) text += chunk;
  return text ? JSON.parse(text) : {};
}

function serveSite(res, host, path) {
  const site = sites.get(host.toLowerCase());
  if (!site) return send(res, 502, { message: `no fake site for ${host}` });
  if (path !== "/.well-known/simpled/version") return send(res, 404, { message: "not found" });
  return send(res, site.status ?? 200, site.body);
}

async function control(req, res, parts) {
  const [kind, ...rest] = parts;
  if (kind === "health") return send(res, 200, "ok", "text/plain");
  if (kind === "tokens" && rest.length === 1 && req.method === "PUT") {
    tokens.set(rest[0], (await readJson(req)).login);
    return send(res, 200, {});
  }
  if (kind === "repos" && rest.length === 2) {
    const key = repoKey(rest[0], rest[1]);
    if (req.method === "DELETE") {
      repos.delete(key);
      return send(res, 200, {});
    }
    if (req.method === "PUT") {
      const body = await readJson(req);
      repos.set(key, {
        owner: rest[0],
        name: rest[1],
        defaultBranch: body.defaultBranch ?? "main",
        tokens: new Set(body.tokens ?? []),
        files: body.files ?? {},
      });
      return send(res, 200, {});
    }
  }
  if (kind === "sites" && rest.length === 1) {
    const host = rest[0].toLowerCase();
    if (req.method === "DELETE") {
      sites.delete(host);
      return send(res, 200, {});
    }
    if (req.method === "PUT") {
      const body = await readJson(req);
      sites.set(host, { status: body.status, body: body.body });
      return send(res, 200, {});
    }
  }
  return send(res, 404, { message: "unknown fakes call" });
}

function github(req, res, url) {
  const token = (req.headers.authorization ?? "").replace(/^Bearer /, "");
  if (!tokens.has(token)) return send(res, 401, { message: "Bad credentials" });
  const parts = url.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  if (parts.length === 1 && parts[0] === "user") return send(res, 200, { login: tokens.get(token) });
  if (parts[0] !== "repos" || parts.length < 3) return send(res, 404, { message: "Not Found" });

  const repo = repos.get(repoKey(parts[1], parts[2]));
  if (!repo || !repo.tokens.has(token)) return send(res, 404, { message: "Not Found" });
  const rest = parts.slice(3);
  if (rest.length === 0) {
    return send(res, 200, { name: repo.name, owner: { login: repo.owner }, default_branch: repo.defaultBranch });
  }
  if (rest[0] === "git" && rest[1] === "trees") {
    const paths = Object.keys(repo.files).sort();
    const sha = createHash("sha1").update(JSON.stringify(repo.files)).digest("hex");
    return send(res, 200, { sha, truncated: false, tree: paths.map((path) => ({ path, type: "blob" })) });
  }
  if (rest[0] === "contents") {
    const text = repo.files[rest.slice(1).join("/")];
    if (text === undefined) return send(res, 404, { message: "Not Found" });
    return send(res, 200, text, "text/plain");
  }
  return send(res, 404, { message: "Not Found" });
}

const server = createServer(async (req, res) => {
  try {
    if (/^https?:\/\//.test(req.url)) {
      const target = new URL(req.url);
      return serveSite(res, target.host, target.pathname);
    }
    const url = new URL(req.url, "http://fakes");
    if (url.pathname.startsWith("/_fakes/")) {
      return await control(req, res, url.pathname.split("/").filter(Boolean).slice(1).map(decodeURIComponent));
    }
    return github(req, res, url);
  } catch (e) {
    return send(res, 500, { message: String(e) });
  }
});

server.listen(3000, () => console.log("simpled-fakes listening on :3000"));
