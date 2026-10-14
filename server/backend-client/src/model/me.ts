import { z } from "zod";
import { OrgRole } from "./common";

export const Account = z.object({
  id: z.string(),
  email: z.string(),
  name: z.string(),
});
export type Account = z.output<typeof Account>;

/** An organization the caller belongs to, with their role in it. */
export const MyOrganization = z.object({
  id: z.number(),
  name: z.string(),
  slug: z.string(),
  role: OrgRole,
});
export type MyOrganization = z.output<typeof MyOrganization>;

export const Me = z.object({
  account: Account,
  organizations: z.array(MyOrganization),
});
export type Me = z.output<typeof Me>;

/** `{token}` for `?useToken=true` sign-ins; the CLI keeps it as its bearer. */
export const SessionToken = z.object({ token: z.string() });
export type SessionToken = z.output<typeof SessionToken>;

export interface Registration {
  email: string;
  password: string;
  name: string;
  organizationName: string;
}
