import { z } from "zod";
import { isoDate, OrgRole } from "./common";

/** What the caller may do in the organization; the UI hides the rest. */
export const OrgPermissions = z.object({
  canManageOrganization: z.boolean(),
  canManageMembers: z.boolean(),
  canCreateTeam: z.boolean(),
  canCreateProject: z.boolean(),
  /** Add, rotate and remove the organization's GitHub tokens. */
  canManageGitHubConnections: z.boolean(),
  /** Roles the caller may invite with or assign. */
  assignableRoles: z.array(OrgRole),
});
export type OrgPermissions = z.output<typeof OrgPermissions>;

export const Organization = z.object({
  id: z.number(),
  name: z.string(),
  slug: z.string(),
  createdAt: isoDate,
  /** The caller's role. */
  role: OrgRole,
  permissions: OrgPermissions,
});
export type Organization = z.output<typeof Organization>;

export const Member = z.object({
  accountId: z.string(),
  email: z.string(),
  name: z.string(),
  role: OrgRole,
  joinedAt: isoDate,
  isMe: z.boolean(),
  /** False for the last owner, whatever the caller's role. */
  canChangeRole: z.boolean(),
  canRemove: z.boolean(),
});
export type Member = z.output<typeof Member>;
