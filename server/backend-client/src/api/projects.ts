import { z } from "zod";
import type { ApiContext } from "../context";
import {
  AlreadyMember,
  type ApiNotAuthorized,
  ErrorCode,
  type InvalidFormField,
  OrganizationNotFound,
  ProjectNotFound,
  type RoleForbidden,
  TeamNotFound,
  type TransportError,
  UnexpectedApiError,
  ValidationError,
} from "../errors";
import { parse } from "../model/common";
import { type NewProject, Project, type ProjectUpdate } from "../model/project";
import { forward, type Result } from "../result";
import type { Transport } from "../transport";
import { asForbidden, asFormField, signedIn } from "./common";

/**
 * Projects of an organization, linked to teams. Owners and admins manage every project; a manager manages the
 * projects of their teams, and links a team to a project only when they are in both.
 */
export class ProjectsApi {
  constructor(private readonly t: Transport) {}

  async list(ctx: ApiContext, orgId: number): Promise<Result<Project[], ApiNotAuthorized | OrganizationNotFound>> {
    const r = await this.t.get(ctx, `/Orgs/${orgId}/Projects`);
    return forward(
      r,
      (j) => parse(z.array(Project), j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  /**
   * Owners, admins and managers. A manager's project must link only teams they are in, at least one; otherwise
   * `RoleForbidden` with `fieldName` `teamIds`. Unknown teams are `InvalidFormField` on `teamIds`.
   */
  async create(
    ctx: ApiContext,
    orgId: number,
    project: NewProject,
  ): Promise<Result<Project, ApiNotAuthorized | OrganizationNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.post(ctx, `/Orgs/${orgId}/Projects`, {
      body: { name: project.name, slug: project.slug ?? null, teamIds: project.teamIds ?? [] },
    });
    return forward(
      r,
      (j) => parse(Project, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "orgId") return new OrganizationNotFound(orgId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  async get(ctx: ApiContext, projectId: number): Promise<Result<Project, ApiNotAuthorized | ProjectNotFound>> {
    const r = await this.t.get(ctx, `/Projects/${projectId}`);
    return forward(
      r,
      (j) => parse(Project, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "projectId") return new ProjectNotFound(projectId, e);
          throw new UnexpectedApiError(e);
        }
        return signedIn(e);
      },
    );
  }

  async update(
    ctx: ApiContext,
    projectId: number,
    update: ProjectUpdate,
  ): Promise<Result<Project, ApiNotAuthorized | ProjectNotFound | RoleForbidden | InvalidFormField>> {
    const r = await this.t.put(ctx, `/Projects/${projectId}`, { body: update });
    return forward(
      r,
      (j) => parse(Project, j),
      (e) => {
        if (e instanceof ValidationError) {
          if (e.incorrectValue === "projectId") return new ProjectNotFound(projectId, e);
          return asForbidden(e) ?? asFormField(e);
        }
        return signedIn(e);
      },
    );
  }

  async delete(
    ctx: ApiContext,
    projectId: number,
  ): Promise<Result<void, ApiNotAuthorized | ProjectNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Projects/${projectId}`);
    return forward(r, () => undefined, manageFailure(projectId));
  }

  /** `AlreadyMember` when the team is already linked. */
  async linkTeam(
    ctx: ApiContext,
    projectId: number,
    teamId: number,
  ): Promise<Result<void, ApiNotAuthorized | ProjectNotFound | TeamNotFound | AlreadyMember | RoleForbidden>> {
    const r = await this.t.post(ctx, `/Projects/${projectId}/Teams/${teamId}`);
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError && e.incorrectValue === "teamId") {
          return e.code === ErrorCode.AlreadyExist ? new AlreadyMember(e.details, e) : new TeamNotFound(teamId, e);
        }
        return manageFailure(projectId)(e);
      },
    );
  }

  /** `TeamNotFound` also when the team is not linked. */
  async unlinkTeam(
    ctx: ApiContext,
    projectId: number,
    teamId: number,
  ): Promise<Result<void, ApiNotAuthorized | ProjectNotFound | TeamNotFound | RoleForbidden>> {
    const r = await this.t.delete(ctx, `/Projects/${projectId}/Teams/${teamId}`);
    return forward(
      r,
      () => undefined,
      (e) => {
        if (e instanceof ValidationError && e.incorrectValue === "teamId") return new TeamNotFound(teamId, e);
        return manageFailure(projectId)(e);
      },
    );
  }
}

function manageFailure(projectId: number) {
  return (e: TransportError): ApiNotAuthorized | ProjectNotFound | RoleForbidden => {
    if (e instanceof ValidationError) {
      if (e.incorrectValue === "projectId") return new ProjectNotFound(projectId, e);
      const forbidden = asForbidden(e);
      if (forbidden) return forbidden;
      throw new UnexpectedApiError(e);
    }
    return signedIn(e);
  };
}
