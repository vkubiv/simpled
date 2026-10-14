import { z } from "zod";
import { isoDate, OrgRole } from "./common";

export const TeamSummary = z.object({
  id: z.number(),
  name: z.string(),
  memberCount: z.number(),
  projectCount: z.number(),
  /** The caller is in the team. */
  isMember: z.boolean(),
  canManage: z.boolean(),
});
export type TeamSummary = z.output<typeof TeamSummary>;

export const TeamMember = z.object({
  accountId: z.string(),
  email: z.string(),
  name: z.string(),
  /** Their role in the organization. */
  role: OrgRole,
});
export type TeamMember = z.output<typeof TeamMember>;

export const ProjectRef = z.object({
  id: z.number(),
  name: z.string(),
  slug: z.string(),
});
export type ProjectRef = z.output<typeof ProjectRef>;

export const Team = z.object({
  id: z.number(),
  organizationId: z.number(),
  name: z.string(),
  createdAt: isoDate,
  members: z.array(TeamMember),
  projects: z.array(ProjectRef),
  isMember: z.boolean(),
  canManage: z.boolean(),
});
export type Team = z.output<typeof Team>;
