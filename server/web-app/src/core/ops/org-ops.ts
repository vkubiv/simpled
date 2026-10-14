import {
  type ApiContext,
  type ApiNotAuthorized,
  type InvalidFormField,
  type LastOwner,
  type Member,
  type MemberNotFound,
  type Organization,
  OrganizationNotFound,
  type OrgRole,
  type RoleForbidden,
} from "@simpled/backend-client";
import { type AsyncResult, ok } from "operation-result.js";
import type { AuthHandler } from "trunk.js";
import type { AccountSession } from "../auth/account-session";
import { errorText } from "../primitives/errors";
import type { MeRepository } from "../repositories/me-repository";
import type { OrgRepository } from "../repositories/org-repository";
import type { BackendApi } from "./api";
import { refreshMe } from "./auth-ops";

type CallDeps = { client: BackendApi; authHandler: AuthHandler<ApiContext> };
type MeDeps = CallDeps & { session: AccountSession; meRepository: MeRepository };

/** Loads the organization the signed-in area shows; `notFound` for one the caller is not (or no longer) in. */
export async function loadOrg(o: CallDeps & { orgRepository: OrgRepository; orgId: number }): Promise<void> {
  o.orgRepository.set({ kind: "loading", orgId: o.orgId });
  const r = await o.authHandler.callApi((ctx) => o.client.orgs.get(ctx, o.orgId));
  const current = o.orgRepository.status;
  // A newer load for another organization started meanwhile: its answer wins.
  if (current.kind === "loading" && current.orgId !== o.orgId) return;
  if (r.ok) o.orgRepository.set({ kind: "loaded", org: r.value });
  else if (r.errors.some((e) => e instanceof OrganizationNotFound))
    o.orgRepository.set({ kind: "notFound", orgId: o.orgId });
  else o.orgRepository.set({ kind: "failed", orgId: o.orgId, message: errorText(r.errors) });
}

export async function createOrg(
  o: MeDeps & { name: string },
): AsyncResult<Organization, ApiNotAuthorized | InvalidFormField> {
  const r = await o.authHandler.callApi((ctx) => o.client.orgs.create(ctx, o.name.trim()));
  if (r.ok) await refreshMe(o);
  return r;
}

export async function renameOrg(
  o: MeDeps & { orgRepository: OrgRepository; orgId: number; name: string },
): AsyncResult<Organization, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField> {
  const r = await o.authHandler.callApi((ctx) => o.client.orgs.rename(ctx, o.orgId, o.name.trim()));
  if (!r.ok) return r;
  o.orgRepository.set({ kind: "loaded", org: r.value });
  await refreshMe(o);
  return r;
}

export async function deleteOrg(
  o: MeDeps & { orgRepository: OrgRepository; orgId: number },
): AsyncResult<void, ApiNotAuthorized | OrganizationNotFound | RoleForbidden> {
  const r = await o.authHandler.callApi((ctx) => o.client.orgs.delete(ctx, o.orgId));
  if (!r.ok) return r;
  o.orgRepository.reset();
  await refreshMe(o);
  return ok();
}

export function loadMembers(
  o: CallDeps & { orgId: number },
): AsyncResult<Member[], ApiNotAuthorized | OrganizationNotFound> {
  return o.authHandler.callApi((ctx) => o.client.orgs.members(ctx, o.orgId));
}

export function setMemberRole(
  o: CallDeps & { orgId: number; accountId: string; role: OrgRole },
): AsyncResult<void, ApiNotAuthorized | OrganizationNotFound | MemberNotFound | RoleForbidden | LastOwner> {
  return o.authHandler.callApi((ctx) => o.client.orgs.setRole(ctx, o.orgId, o.accountId, o.role));
}

/**
 * Removes a member. When it is the caller leaving, their organizations are reloaded and the shown organization is
 * dropped; the page then goes home.
 */
export async function removeMember(
  o: MeDeps & { orgRepository: OrgRepository; orgId: number; accountId: string; isMe: boolean },
): AsyncResult<void, ApiNotAuthorized | OrganizationNotFound | MemberNotFound | RoleForbidden | LastOwner> {
  const r = await o.authHandler.callApi((ctx) => o.client.orgs.removeMember(ctx, o.orgId, o.accountId));
  if (!r.ok || !o.isMe) return r;
  o.orgRepository.reset();
  await refreshMe(o);
  return r;
}
