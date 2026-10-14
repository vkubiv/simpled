import { z } from "zod";
import { isoDate } from "./common";

export const TeamRef = z.object({
  id: z.number(),
  name: z.string(),
});
export type TeamRef = z.output<typeof TeamRef>;

export const Project = z.object({
  id: z.number(),
  organizationId: z.number(),
  name: z.string(),
  slug: z.string(),
  createdAt: isoDate,
  teams: z.array(TeamRef),
  /** The caller is in one of the project's teams. */
  isMember: z.boolean(),
  canManage: z.boolean(),
});
export type Project = z.output<typeof Project>;

export interface NewProject {
  name: string;
  /** Made from the name when omitted. */
  slug?: string;
  /** A manager must give at least one, and only teams they are in. */
  teamIds?: number[];
}

export interface ProjectUpdate {
  name: string;
  slug: string;
}
