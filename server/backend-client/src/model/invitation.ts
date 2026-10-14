import { z } from "zod";
import { isoDate, OrgRole } from "./common";

export const Invitation = z.object({
  id: z.number(),
  email: z.string(),
  role: OrgRole,
  invitedBy: z.string(),
  createdAt: isoDate,
  expiresAt: isoDate,
  /** Expired invitations stay listed until revoked or replaced by a new one. */
  expired: z.boolean(),
});
export type Invitation = z.output<typeof Invitation>;

/** What the invitation page shows before the person accepts or registers. */
export const InvitationPreview = z.object({
  organizationName: z.string(),
  role: OrgRole,
  email: z.string(),
  invitedBy: z.string(),
  /** True: sign in and accept. False: register through the invitation. */
  accountExists: z.boolean(),
});
export type InvitationPreview = z.output<typeof InvitationPreview>;
