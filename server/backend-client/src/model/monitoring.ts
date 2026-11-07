import { z } from "zod";
import { isoDate } from "./common";

/** A GitHub personal access token of the organization; the token itself never comes back. */
export const GitHubConnection = z.object({
  id: z.number(),
  name: z.string(),
  /** The GitHub account the token belongs to. */
  gitHubLogin: z.string(),
  createdAt: isoDate,
  repositoryCount: z.number(),
});
export type GitHubConnection = z.output<typeof GitHubConnection>;

export interface NewGitHubConnection {
  name: string;
  token: string;
}

export interface GitHubConnectionUpdate {
  name: string;
  /** A new token; omitted keeps the current one. */
  token?: string;
}

/**
 * What the monitor last saw at the deployment's version endpoint (on Docker, the simpled controller's, with its
 * health): `unknown` before the first check, `down` after failed checks in a row or when the controller reports it
 * down, `degraded` when the controller reports it so twice in a row, `mismatch` when the endpoint answers for
 * another application or deployment.
 */
export const DEPLOYMENT_STATUSES = ["unknown", "up", "degraded", "down", "mismatch"] as const;
export const DeploymentStatus = z.enum(DEPLOYMENT_STATUSES);
export type DeploymentStatus = z.output<typeof DeploymentStatus>;

export const Deployment = z.object({
  id: z.number(),
  name: z.string(),
  applicationName: z.string(),
  /** The envspec's `application.version`, such as `^1.2.0`. */
  versionConstraint: z.string().nullable(),
  /** The primary host, the site itself; null when the envspec gives none. */
  url: z.string().nullable(),
  /** The appspec's version on the default branch: what ships next. */
  repoVersion: z.string().nullable(),
  status: DeploymentStatus,
  statusSince: isoDate.nullable(),
  liveVersion: z.string().nullable(),
  liveDeployedAt: isoDate.nullable(),
  checkedAt: isoDate.nullable(),
  checkError: z.string().nullable(),
});
export type Deployment = z.output<typeof Deployment>;

/** One envspec of a repository. */
export const Environment = z.object({
  id: z.number(),
  name: z.string(),
  path: z.string(),
  deployments: z.array(Deployment),
});
export type Environment = z.output<typeof Environment>;

export const Repository = z.object({
  id: z.number(),
  projectId: z.number(),
  connectionId: z.number(),
  owner: z.string(),
  name: z.string(),
  defaultBranch: z.string(),
  createdAt: isoDate,
  syncedAt: isoDate.nullable(),
  /** Why the last sync failed, or which spec files it could not read. */
  syncError: z.string().nullable(),
  environments: z.array(Environment),
  /** Sync and unlink. */
  canManage: z.boolean(),
});
export type Repository = z.output<typeof Repository>;

export interface RepositoryLink {
  connectionId: number;
  /** `owner/name`, or the repository's GitHub URL. */
  fullName: string;
}

export const DEPLOYMENT_EVENT_KINDS = ["version-changed", "down", "degraded", "up", "mismatch"] as const;
export const DeploymentEventKind = z.enum(DEPLOYMENT_EVENT_KINDS);
export type DeploymentEventKind = z.output<typeof DeploymentEventKind>;

export const DeploymentEvent = z.object({
  id: z.number(),
  at: isoDate,
  kind: DeploymentEventKind,
  version: z.string().nullable(),
  previousVersion: z.string().nullable(),
  details: z.string().nullable(),
});
export type DeploymentEvent = z.output<typeof DeploymentEvent>;
