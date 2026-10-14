import type { OrgRole } from "@simpled/backend-client";

export const ROLE_LABELS: Record<OrgRole, string> = {
  owner: "Owner",
  admin: "Admin",
  manager: "Manager",
  member: "Member",
};

export const ROLE_DESCRIPTIONS: Record<OrgRole, string> = {
  owner: "Everything, including the organization's settings and its owners.",
  admin: "All teams and projects; invites and removes members.",
  manager: "Creates teams and projects; manages the ones they belong to.",
  member: "Views the organization.",
};
