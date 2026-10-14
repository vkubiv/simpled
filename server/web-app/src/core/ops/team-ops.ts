import type {
  AlreadyMember,
  ApiContext,
  ApiNotAuthorized,
  InvalidFormField,
  MemberNotFound,
  OrganizationNotFound,
  RoleForbidden,
  Team,
  TeamNotFound,
  TeamSummary,
} from "@simpled/backend-client";
import type { AsyncResult } from "operation-result.js";
import type { AuthHandler } from "trunk.js";
import type { BackendApi } from "./api";

type CallDeps = { client: BackendApi; authHandler: AuthHandler<ApiContext> };

export function loadTeams(
  o: CallDeps & { orgId: number },
): AsyncResult<TeamSummary[], ApiNotAuthorized | OrganizationNotFound> {
  return o.authHandler.callApi((ctx) => o.client.teams.list(ctx, o.orgId));
}

export function createTeam(
  o: CallDeps & { orgId: number; name: string },
): AsyncResult<Team, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField> {
  return o.authHandler.callApi((ctx) => o.client.teams.create(ctx, o.orgId, o.name.trim()));
}

export function loadTeam(o: CallDeps & { teamId: number }): AsyncResult<Team, ApiNotAuthorized | TeamNotFound> {
  return o.authHandler.callApi((ctx) => o.client.teams.get(ctx, o.teamId));
}

export function renameTeam(
  o: CallDeps & { teamId: number; name: string },
): AsyncResult<Team, ApiNotAuthorized | TeamNotFound | RoleForbidden | InvalidFormField> {
  return o.authHandler.callApi((ctx) => o.client.teams.rename(ctx, o.teamId, o.name.trim()));
}

export function deleteTeam(
  o: CallDeps & { teamId: number },
): AsyncResult<void, ApiNotAuthorized | TeamNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.teams.delete(ctx, o.teamId));
}

export function addTeamMember(
  o: CallDeps & { teamId: number; accountId: string },
): AsyncResult<void, ApiNotAuthorized | TeamNotFound | MemberNotFound | AlreadyMember | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.teams.addMember(ctx, o.teamId, o.accountId));
}

export function removeTeamMember(
  o: CallDeps & { teamId: number; accountId: string },
): AsyncResult<void, ApiNotAuthorized | TeamNotFound | MemberNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.teams.removeMember(ctx, o.teamId, o.accountId));
}
