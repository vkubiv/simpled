import { z } from "zod";
import type { ApiContext } from "../context";
import {
  AlreadyMember,
  type ApiNotAuthorized,
  ErrorCode,
  type InvalidFormField,
  MemberNotFound,
  OrganizationNotFound,
  type RoleForbidden,
  TeamNotFound,
  type TransportError,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { parse } from "../model/common";
import { Team, TeamSummary } from "../model/team";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, signedIn } from "./common";

/** Teams of an organization. Owners and admins manage every team, managers the teams they are in. */
export class TeamsApi {
  constructor(private readonly t: Transport) {}

  async list(ctx: ApiContext, orgId: number): Promise<Result<TeamSummary[], ApiNotAuthorized | OrganizationNotFound>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}/Teams`);
    return forward(
      r,
      (j) => parse(z.array(TeamSummary), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Owners, admins and managers; a manager joins the team they create. */
  async create(
    ctx: ApiContext,
    orgId: number,
    name: string,
  ): Promise<Result<Team, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.post(ctx, `/Orgs/${orgId}/Teams`, { body: { name } });
    return forward(
      r,
      (j) => parse(Team, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  async get(ctx: ApiContext, teamId: number): Promise<Result<Team, ApiNotAuthorized | TeamNotFound>> {
    const r = await this.t.get(ctx, `/Teams/${teamId}`);
    return forward(
      r,
      (j) => parse(Team, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "teamId") return new TeamNotFound(teamId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  async rename(
    ctx: ApiContext,
    teamId: number,
    name: string,
  ): Promise<Result<Team, ApiNotAuthorized | TeamNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.put(ctx, `/Teams/${teamId}`, { body: { name } });
    return forward(
      r,
      (j) => parse(Team, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "teamId") return new TeamNotFound(teamId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  /** Deletes the team; its projects stay, unlinked from it. */
  async delete(
    ctx: ApiContext,
    teamId: number,
  ): Promise<Result<void, ApiNotAuthorized | TeamNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Teams/${teamId}`);
    return forward(r, () => undefined, manageFailure(teamId));
  }

  /** Adds an organization member to the team. */
  async addMember(
    ctx: ApiContext,
    teamId: number,
    accountId: string,
  ): Promise<Result<void, ApiNotAuthorized | TeamNotFound | MemberNotFound | AlreadyMember | RoleForbidden>> {
    const r = await this.t.post(ctx, `/Teams/${teamId}/Members/${accountId}`);
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError && e.incorrectValue === "accountId") {
          return e.code === ErrorCode.AlreadyExist ? new AlreadyMember(e.details, e) : new MemberNotFound(accountId, e);
        }
        return manageFailure(teamId)(e);
      },
    );
  }

  async removeMember(
    ctx: ApiContext,
    teamId: number,
    accountId: string,
  ): Promise<Result<void, ApiNotAuthorized | TeamNotFound | MemberNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Teams/${teamId}/Members/${accountId}`);
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError && e.incorrectValue === "accountId") return new MemberNotFound(accountId, e);
        return manageFailure(teamId)(e);
      },
    );
  }
}

function manageFailure(teamId: number) {
  return (e: TransportError): ApiNotAuthorized | TeamNotFound | RoleForbidden => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "teamId") return new TeamNotFound(teamId, e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}
