import { z } from "zod";
import type { ApiContext } from "../context";
import {
  AccountAlreadyExists,
  AlreadyMember,
  type ApiNotAuthorized,
  ErrorCode,
  type InvalidFormField,
  InvalidLink,
  InvitationForAnotherEmail,
  InvitationNotFound,
  OrganizationNotFound,
  type RateLimited,
  type RoleForbidden,
  type TransportError,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { type OrgRole, parse } from "../model/common";
import { Invitation, InvitationPreview } from "../model/invitation";
import { MyOrganization } from "../model/me";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, rateLimited, signedIn } from "./common";

/**
 * Email invitations. Owners and admins manage them inside an organization; the invited person opens the mailed
 * link (`invitation?token=`) and either accepts (signed in with that email) or registers.
 */
export class InvitationsApi {
  constructor(private readonly t: Transport) {}

  /** Mails an invitation; an open one to the same email is replaced. */
  async invite(
    ctx: ApiContext,
    orgId: number,
    email: string,
    role: OrgRole,
  ): Promise<
    Result<Invitation, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | AlreadyMember | InvalidFormField>
  > {
    const r = await this.t.post(ctx, `/Orgs/${orgId}/Invitations`, { body: { email, role } });
    return forward(
      r,
      (j) => parse(Invitation, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          const forbidden = asForbidden(e);
          if (forbidden) return forbidden;
          if (e.code === ErrorCode.AlreadyExist) return new AlreadyMember(e.details, e);
          return asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Open invitations, newest first; expired ones are flagged. Owners and admins. */
  async list(
    ctx: ApiContext,
    orgId: number,
  ): Promise<Result<Invitation[], ApiNotAuthorized | OrganizationNotFound | RoleForbidden>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}/Invitations`);
    return forward(
      r,
      (j) => parse(z.array(Invitation), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          const forbidden = asForbidden(e);
          if (forbidden) return forbidden;
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  async revoke(
    ctx: ApiContext,
    orgId: number,
    invitationId: number,
  ): Promise<Result<void, ApiNotAuthorized | OrganizationNotFound | InvitationNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Orgs/${orgId}/Invitations/${invitationId}`);
    return forward(r, () => undefined, manageFailure(orgId, invitationId));
  }

  /** Anonymous: what the invitation page shows. */
  async preview(ctx: ApiContext, token: string): Promise<Result<InvitationPreview, InvalidLink | RateLimited>> {
    const r = await this.t.post(ctx, "/Invitations/Preview", { body: { token } });
    return forward(
      r,
      (j) => parse(InvitationPreview, j),
      (e) => (e instanceof ValidationError && e.incorrectValue === "token" ? new InvalidLink(e) : rateLimited(e)),
    );
  }

  /** Joins the signed-in account, whose email must be the invited one. */
  async accept(
    ctx: ApiContext,
    token: string,
  ): Promise<Result<MyOrganization, ApiNotAuthorized | InvalidLink | InvitationForAnotherEmail | AlreadyMember>> {
    const r = await this.t.post(ctx, "/Invitations/Accept", { body: { token } });
    return forward(
      r,
      (j) => parse(MyOrganization, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.code === ErrorCode.IncorrectValue) return new InvalidLink(e);
          if (e.code === ErrorCode.IncorrectState) return new InvitationForAnotherEmail(e.details, e);
          if (e.code === ErrorCode.AlreadyExist) return new AlreadyMember(e.details, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Creates the invited person's account (confirmed: the link proves the email), joins, and signs in. */
  async register(
    ctx: ApiContext,
    token: string,
    name: string,
    password: string,
  ): Promise<Result<void, InvalidLink | AccountAlreadyExists | InvalidFormField | RateLimited>> {
    const r = await this.t.post(ctx, "/Invitations/Register", { body: { token, name, password } });
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "token") return new InvalidLink(e);
          if (e.code === ErrorCode.AlreadyExist) return new AccountAlreadyExists(e);
          return asFormField(e);
        }
        return rateLimited(e);
      },
    );
  }
}

function manageFailure(orgId: number, invitationId: number) {
  return (e: TransportError) => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
      if (e.incorrectValue === "invitationId") return new InvitationNotFound(invitationId, e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}
