import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

// `simpled test e2e` writes e2e/.env for the stack it brought up. Reading it for keys the shell did not set lets
// a bare `npx vitest run <file>` reuse the last stack's settings.
const dotenv = join(import.meta.dirname, "..", "..", ".env");
if (existsSync(dotenv)) {
  for (const line of readFileSync(dotenv, "utf8").split(/\r?\n/)) {
    const m = line.match(/^([A-Z0-9_]+)=(.*)$/);
    if (m && process.env[m[1]] === undefined) process.env[m[1]] = m[2];
  }
}

/** Defaults match instance 0 of `just infra` / `just run`: the gateway on :7080, the mock mailer on :7025, fakes on :7010. */
export const env = {
  apiUrl: process.env.E2E_API_URL ?? "http://localhost:7080/api",
  /** sendgrid-mock: every email the backend sends, `GET /api/mails?to=`. */
  mailerUrl: process.env.E2E_MAILER_URL ?? "http://localhost:7025",
  /** simpled-fakes: GitHub and the monitored deployments, scripted through `/_fakes`. */
  fakesUrl: process.env.E2E_FAKES_URL ?? "http://localhost:7010",
  printLogs: process.env.PRINT_LOGS === "1",
};
