import type {
  AlreadyMember,
  ApiContext,
  ApiNotAuthorized,
  InvalidFormField,
  NewProject,
  OrganizationNotFound,
  Project,
  ProjectNotFound,
  ProjectUpdate,
  RoleForbidden,
  TeamNotFound,
} from "@simpled/backend-client";
import type { AsyncResult } from "operation-result.js";
import type { AuthHandler } from "trunk.js";
import type { BackendApi } from "./api";

type CallDeps = { client: BackendApi; authHandler: AuthHandler<ApiContext> };

export function loadProjects(
  o: CallDeps & { orgId: number },
): AsyncResult<Project[], ApiNotAuthorized | OrganizationNotFound> {
  return o.authHandler.callApi((ctx) => o.client.projects.list(ctx, o.orgId));
}

/** An empty slug lets main-svc make one from the name. */
export function createProject(
  o: CallDeps & { orgId: number; project: NewProject },
): AsyncResult<Project, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField> {
  const slug = o.project.slug?.trim();
  return o.authHandler.callApi((ctx) =>
    o.client.projects.create(ctx, o.orgId, { ...o.project, name: o.project.name.trim(), slug: slug || undefined }),
  );
}

export function loadProject(
  o: CallDeps & { projectId: number },
): AsyncResult<Project, ApiNotAuthorized | ProjectNotFound> {
  return o.authHandler.callApi((ctx) => o.client.projects.get(ctx, o.projectId));
}

export function updateProject(
  o: CallDeps & { projectId: number; update: ProjectUpdate },
): AsyncResult<Project, ApiNotAuthorized | ProjectNotFound | RoleForbidden | InvalidFormField> {
  const update = { name: o.update.name.trim(), slug: o.update.slug.trim() };
  return o.authHandler.callApi((ctx) => o.client.projects.update(ctx, o.projectId, update));
}

export function deleteProject(
  o: CallDeps & { projectId: number },
): AsyncResult<void, ApiNotAuthorized | ProjectNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.projects.delete(ctx, o.projectId));
}

export function linkTeam(
  o: CallDeps & { projectId: number; teamId: number },
): AsyncResult<void, ApiNotAuthorized | ProjectNotFound | TeamNotFound | AlreadyMember | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.projects.linkTeam(ctx, o.projectId, o.teamId));
}

export function unlinkTeam(
  o: CallDeps & { projectId: number; teamId: number },
): AsyncResult<void, ApiNotAuthorized | ProjectNotFound | TeamNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.projects.unlinkTeam(ctx, o.projectId, o.teamId));
}
