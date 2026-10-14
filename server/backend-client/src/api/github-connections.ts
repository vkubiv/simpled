import { z } from "zod";
import type { ApiContext } from "../context";
import {
  type ApiNotAuthorized,
  ConnectionInUse,
  ErrorCode,
  GitHubConnectionNotFound,
  type InvalidFormField,
  OrganizationNotFound,
  type RoleForbidden,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { parse } from "../model/common";
import { GitHubConnection, type GitHubConnectionUpdate, type NewGitHubConnection } from "../model/monitoring";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, signedIn } from "./common";

/**
 * An organization's GitHub personal access tokens, which linked repositories are read with. Every member sees
 * them (never the tokens); owners and admins add, rotate and remove them.
 */
export class GitHubConnectionsApi {
  constructor(private readonly t: Transport) {}

  async list(
    ctx: ApiContext,
    orgId: number,
  ): Promise<Result<GitHubConnection[], ApiNotAuthorized | OrganizationNotFound>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}/GitHubConnections`);
    return forward(
      r,
      (j) => parse(z.array(GitHubConnection), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /** The token is checked with GitHub: one it refuses, or GitHub being unreachable, is `InvalidFormField` on `token`. */
  async create(
    ctx: ApiContext,
    orgId: number,
    connection: NewGitHubConnection,
  ): Promise<Result<GitHubConnection, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.post(ctx, `/Orgs/${orgId}/GitHubConnections`, { body: connection });
    return forward(
      r,
      (j) => parse(GitHubConnection, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Renames, and replaces the token when one is given. */
  async update(
    ctx: ApiContext,
    connectionId: number,
    update: GitHubConnectionUpdate,
  ): Promise<Result<GitHubConnection, ApiNotAuthorized | GitHubConnectionNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.put(ctx, `/GitHubConnections/${connectionId}`, {
      body: { name: update.name, token: update.token ?? null },
    });
    return forward(
      r,
      (j) => parse(GitHubConnection, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "connectionId") return new GitHubConnectionNotFound(connectionId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  /** `ConnectionInUse` while repositories are linked through it. */
  async delete(
    ctx: ApiContext,
    connectionId: number,
  ): Promise<Result<void, ApiNotAuthorized | GitHubConnectionNotFound | RoleForbidden | ConnectionInUse>> {
    const r = await this.t.delete(ctx, `/GitHubConnections/${connectionId}`);
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "connectionId") {
            return e.code === ErrorCode.IncorrectState
              ? new ConnectionInUse(e.details, e)
              : new GitHubConnectionNotFound(connectionId, e);
          }
          const forbidden = asForbidden(e);
          if (forbidden) return forbidden;
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }
}
