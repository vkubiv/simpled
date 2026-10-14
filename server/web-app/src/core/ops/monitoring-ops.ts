import type {
  ApiContext,
  ApiNotAuthorized,
  ConnectionInUse,
  DeploymentEvent,
  DeploymentNotFound,
  GitHubConnection,
  GitHubConnectionNotFound,
  GitHubConnectionUpdate,
  InvalidFormField,
  NewGitHubConnection,
  OrganizationNotFound,
  ProjectNotFound,
  Repository,
  RepositoryLink,
  RepositoryNotFound,
  RoleForbidden,
} from "@simpled/backend-client";
import type { AsyncResult } from "operation-result.js";
import type { AuthHandler } from "trunk.js";
import type { BackendApi } from "./api";

type CallDeps = { client: BackendApi; authHandler: AuthHandler<ApiContext> };

export function loadConnections(
  o: CallDeps & { orgId: number },
): AsyncResult<GitHubConnection[], ApiNotAuthorized | OrganizationNotFound> {
  return o.authHandler.callApi((ctx) => o.client.gitHubConnections.list(ctx, o.orgId));
}

export function createConnection(
  o: CallDeps & { orgId: number; connection: NewGitHubConnection },
): AsyncResult<GitHubConnection, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField> {
  const connection = { name: o.connection.name.trim(), token: o.connection.token.trim() };
  return o.authHandler.callApi((ctx) => o.client.gitHubConnections.create(ctx, o.orgId, connection));
}

/** An empty token keeps the current one. */
export function updateConnection(
  o: CallDeps & { connectionId: number; update: GitHubConnectionUpdate },
): AsyncResult<GitHubConnection, ApiNotAuthorized | GitHubConnectionNotFound | RoleForbidden | InvalidFormField> {
  const update = { name: o.update.name.trim(), token: o.update.token?.trim() || undefined };
  return o.authHandler.callApi((ctx) => o.client.gitHubConnections.update(ctx, o.connectionId, update));
}

export function deleteConnection(
  o: CallDeps & { connectionId: number },
): AsyncResult<void, ApiNotAuthorized | GitHubConnectionNotFound | RoleForbidden | ConnectionInUse> {
  return o.authHandler.callApi((ctx) => o.client.gitHubConnections.delete(ctx, o.connectionId));
}

export function loadRepositories(
  o: CallDeps & { projectId: number },
): AsyncResult<Repository[], ApiNotAuthorized | ProjectNotFound> {
  return o.authHandler.callApi((ctx) => o.client.repositories.list(ctx, o.projectId));
}

export function linkRepository(
  o: CallDeps & { projectId: number; link: RepositoryLink },
): AsyncResult<Repository, ApiNotAuthorized | ProjectNotFound | RoleForbidden | InvalidFormField> {
  const link = { connectionId: o.link.connectionId, fullName: o.link.fullName.trim() };
  return o.authHandler.callApi((ctx) => o.client.repositories.link(ctx, o.projectId, link));
}

export function syncRepository(
  o: CallDeps & { repositoryId: number },
): AsyncResult<Repository, ApiNotAuthorized | RepositoryNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.repositories.sync(ctx, o.repositoryId));
}

export function unlinkRepository(
  o: CallDeps & { repositoryId: number },
): AsyncResult<void, ApiNotAuthorized | RepositoryNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.repositories.unlink(ctx, o.repositoryId));
}

export function loadDeploymentEvents(
  o: CallDeps & { deploymentId: number },
): AsyncResult<DeploymentEvent[], ApiNotAuthorized | DeploymentNotFound> {
  return o.authHandler.callApi((ctx) => o.client.repositories.deploymentEvents(ctx, o.deploymentId));
}
