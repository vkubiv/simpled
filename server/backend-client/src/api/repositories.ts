import { z } from "zod";
import type { ApiContext } from "../context";
import {
  type ApiNotAuthorized,
  DeploymentNotFound,
  type InvalidFormField,
  ProjectNotFound,
  RepositoryNotFound,
  type RoleForbidden,
  type TransportError,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { parse } from "../model/common";
import { DeploymentEvent, Repository, type RepositoryLink } from "../model/monitoring";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, signedIn } from "./common";

/**
 * GitHub repositories linked to a project, with the environments and deployments their envspecs declare and
 * what the monitor last saw of each. Every organization member reads them; whoever manages the project links,
 * syncs and unlinks.
 */
export class RepositoriesApi {
  constructor(private readonly t: Transport) {}

  async list(ctx: ApiContext, projectId: number): Promise<Result<Repository[], ApiNotAuthorized | ProjectNotFound>> {
    const r = await this.t.get(ctx, `/Projects/${projectId}/Repositories`);
    return forward(
      r,
      (j) => parse(z.array(Repository), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "projectId") return new ProjectNotFound(projectId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /**
   * Links and syncs at once. `InvalidFormField` on `fullName` for a malformed name, a repository the
   * connection's token cannot read, or one already linked; on `connectionId` for an unknown connection or a
   * token GitHub no longer accepts.
   */
  async link(
    ctx: ApiContext,
    projectId: number,
    link: RepositoryLink,
  ): Promise<Result<Repository, ApiNotAuthorized | ProjectNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.post(ctx, `/Projects/${projectId}/Repositories`, { body: link });
    return forward(
      r,
      (j) => parse(Repository, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "projectId") return new ProjectNotFound(projectId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  async get(ctx: ApiContext, repositoryId: number): Promise<Result<Repository, ApiNotAuthorized | RepositoryNotFound>> {
    const r = await this.t.get(ctx, `/Repositories/${repositoryId}`);
    return forward(r, (j) => parse(Repository, j), readFailure(repositoryId));
  }

  /** Re-reads the spec files now; a failed read is the repository's `syncError`, not an error here. */
  async sync(
    ctx: ApiContext,
    repositoryId: number,
  ): Promise<Result<Repository, ApiNotAuthorized | RepositoryNotFound | RoleForbidden>> {
    const r = await this.t.post(ctx, `/Repositories/${repositoryId}/Sync`);
    return forward(r, (j) => parse(Repository, j), manageFailure(repositoryId));
  }

  async unlink(
    ctx: ApiContext,
    repositoryId: number,
  ): Promise<Result<void, ApiNotAuthorized | RepositoryNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Repositories/${repositoryId}`);
    return forward(r, () => undefined, manageFailure(repositoryId));
  }

  /** What the monitor noticed about a deployment, newest first, at most 50. */
  async deploymentEvents(
    ctx: ApiContext,
    deploymentId: number,
  ): Promise<Result<DeploymentEvent[], ApiNotAuthorized | DeploymentNotFound>> {
    const r = await this.t.get(ctx, `/Deployments/${deploymentId}/Events`);
    return forward(
      r,
      (j) => parse(z.array(DeploymentEvent), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "deploymentId") return new DeploymentNotFound(deploymentId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }
}

function readFailure(repositoryId: number) {
  return (e: TransportError): ApiNotAuthorized | RepositoryNotFound => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "repositoryId") return new RepositoryNotFound(repositoryId, e);
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}

function manageFailure(repositoryId: number) {
  return (e: TransportError): ApiNotAuthorized | RepositoryNotFound | RoleForbidden => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "repositoryId") return new RepositoryNotFound(repositoryId, e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}
