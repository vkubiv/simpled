import { z } from "zod";
import { ParsingError } from "../errors";

/** Parses a response body, raising `ParsingError` with Zod's report when the contract has drifted. */
export function parse<S extends z.ZodType>(schema: S, json: unknown): z.output<S> {
  const r = schema.safeParse(json);
  if (!r.success) throw new ParsingError(z.prettifyError(r.error), r.error);
  return r.data;
}

/** An ISO date-time string on the wire, a `Date` in the model. */
export const isoDate = z.iso.datetime({ offset: true }).transform((s) => new Date(s));

/** A member's role in one organization, strongest first. */
export const ORG_ROLES = ["owner", "admin", "manager", "member"] as const;
export const OrgRole = z.enum(ORG_ROLES);
export type OrgRole = z.output<typeof OrgRole>;
