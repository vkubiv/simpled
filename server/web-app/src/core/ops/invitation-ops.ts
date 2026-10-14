import type {
  AccountAlreadyExists,
  AlreadyMember,
  ApiContext,
  ApiNotAuthorized,
  InvalidFormField,
  InvalidLink,
  Invitation,
  InvitationForAnotherEmail,
  InvitationNotFound,
  InvitationPreview,
  MyOrganization,
  OrganizationNotFound,
  OrgRole,
  RateLimited,
  RoleForbidden,
} from "@simpled/backend-client";
import { type AsyncResult, ok } from "operation-result.js";
import type { AuthHandler } from "trunk.js";
import type { AccountSession } from "../auth/account-session";
import type { MeRepository } from "../repositories/me-repository";
import type { BackendApi } from "./api";
import { refreshMe, restoreSession } from "./auth-ops";

type CallDeps = { client: BackendApi; authHandler: AuthHandler<ApiContext> };

export function invite(
  o: CallDeps & { orgId: number; email: string; role: OrgRole },
): AsyncResult<Invitation, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | AlreadyMember | InvalidFormField> {
  return o.authHandler.callApi((ctx) => o.client.invitations.invite(ctx, o.orgId, o.email.trim(), o.role));
}

export function loadInvitations(
  o: CallDeps & { orgId: number },
): AsyncResult<Invitation[], ApiNotAuthorized | OrganizationNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.invitations.list(ctx, o.orgId));
}

export function revokeInvitation(
  o: CallDeps & { orgId: number; invitationId: number },
): AsyncResult<void, ApiNotAuthorized | OrganizationNotFound | InvitationNotFound | RoleForbidden> {
  return o.authHandler.callApi((ctx) => o.client.invitations.revoke(ctx, o.orgId, o.invitationId));
}

/** Anonymous: works signed in or out. */
export function previewInvitation(o: {
  client: BackendApi;
  session: AccountSession;
  token: string;
}): AsyncResult<InvitationPreview, InvalidLink | RateLimited> {
  return o.client.invitations.preview(o.session.context, o.token);
}

export async function acceptInvitation(
  o: CallDeps & { session: AccountSession; meRepository: MeRepository; token: string },
): AsyncResult<MyOrganization, ApiNotAuthorized | InvalidLink | InvitationForAnotherEmail | AlreadyMember> {
  const r = await o.authHandler.callApi((ctx) => o.client.invitations.accept(ctx, o.token));
  if (r.ok) await refreshMe(o);
  return r;
}

/** Creates the invited person's account and signs in; the app then lands in the organization. */
export async function registerByInvitation(o: {
  client: BackendApi;
  session: AccountSession;
  meRepository: MeRepository;
  token: string;
  name: string;
  password: string;
}): AsyncResult<void, InvalidLink | AccountAlreadyExists | InvalidFormField | RateLimited> {
  const r = await o.client.invitations.register(o.session.context, o.token, o.name.trim(), o.password);
  if (!r.ok) return r;
  await restoreSession(o);
  return ok();
}
