import { z } from "zod";
import type { ApiContext } from "../context";
import {
  type ApiNotAuthorized,
  ErrorCode,
  type InvalidFormField,
  LastOwner,
  MemberNotFound,
  OrganizationNotFound,
  type RoleForbidden,
  type TransportError,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { type OrgRole, parse } from "../model/common";
import { Member, Organization } from "../model/organization";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, signedIn } from "./common";

/** `/Orgs`: organizations and their members. An organization the caller is not in is `OrganizationNotFound`. */
export class OrgsApi {
  constructor(private readonly t: Transport) {}

  /** Another organization, owned by the caller. */
  async create(ctx: ApiContext, name: string): Promise<Result<Organization, ApiNotAuthorized | InvalidFormField>> {
    const r = await this.t.post(ctx, "/Orgs", { body: { name } });
    return forward(
      r,
      (j) => parse(Organization, j),
      (e) => (e instanceof ValidationError ? asFormField(e) : signedIn(e)),
    );
  }

  /** The organization with the caller's role and permissions. */
  async get(ctx: ApiContext, orgId: number): Promise<Result<Organization, ApiNotAuthorized | OrganizationNotFound>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}`);
    return forward(
      r,
      (j) => parse(Organization, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Renames the organization; the slug stays. Owner only. */
  async rename(
    ctx: ApiContext,
    orgId: number,
    name: string,
  ): Promise<Result<Organization, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.put(ctx, `/Orgs/${orgId}`, { body: { name } });
    return forward(
      r,
      (j) => parse(Organization, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Deletes the organization with its members, invitations, teams and projects. Owner only. */
  async delete(
    ctx: ApiContext,
    orgId: number,
  ): Promise<Result<void, ApiNotAuthorized | OrganizationNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Orgs/${orgId}`);
    return forward(r, () => undefined, orgFailure(orgId));
  }

  async members(ctx: ApiContext, orgId: number): Promise<Result<Member[], ApiNotAuthorized | OrganizationNotFound>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}/Members`);
    return forward(
      r,
      (j) => parse(z.array(Member), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Owners change any role; admins any but owner, and never an owner's. The last owner can't be demoted. */
  async setRole(
    ctx: ApiContext,
    orgId: number,
    accountId: string,
    role: OrgRole,
  ): Promise<Result<void, ApiNotAuthorized | OrganizationNotFound | MemberNotFound | RoleForbidden | LastOwner>> {
    const r = await this.t.put(ctx, `/Orgs/${orgId}/Members/${accountId}/Role`, { body: { role } });
    return forward(r, () => undefined, memberFailure(orgId, accountId));
  }

  /** Removes a member (owners and admins), or leaves when `accountId` is the caller's. The last owner stays. */
  async removeMember(
    ctx: ApiContext,
    orgId: number,
    accountId: string,
  ): Promise<Result<void, ApiNotAuthorized | OrganizationNotFound | MemberNotFound | RoleForbidden | LastOwner>> {
    const r = await this.t.delete(ctx, `/Orgs/${orgId}/Members/${accountId}`);
    return forward(r, () => undefined, memberFailure(orgId, accountId));
  }
}

function orgFailure(orgId: number) {
  return (e: TransportError) => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}

function memberFailure(orgId: number, accountId: string) {
  return (e: TransportError) => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
      if (e.incorrectValue === "accountId" && e.code === ErrorCode.DoesntExist) return new MemberNotFound(accountId, e);
      if (e.incorrectValue === "accountId" && e.code === ErrorCode.IncorrectState) return new LastOwner(e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}
