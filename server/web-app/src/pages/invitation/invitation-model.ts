import type { InvitationPreview } from "@simpled/backend-client";
import { Rx } from "subtree.js/core";
import type { FieldErrors } from "../../core/primitives/forms";
import { type FormStatus, idleForm, idleLoad, type LoadStatus } from "../../core/primitives/load-status";

/**
 * What the invited person can do here: accept as the signed-in account, switch accounts (signed in as someone
 * else), sign in first (the email has an account), or register (it has none).
 */
export type InvitationMode = "accept" | "wrongAccount" | "signIn" | "register";

export interface InvitationDraft {
  readonly name: string;
  readonly password: string;
}

export class InvitationState {
  readonly load = new Rx<LoadStatus>(idleLoad);
  readonly preview = new Rx<InvitationPreview | null>(null);
  readonly signedInEmail = new Rx<string | null>(null);
  readonly mode = new Rx<InvitationMode | null>(null);
  readonly draft = new Rx<InvitationDraft>({ name: "", password: "" });
  readonly fieldErrors = new Rx<FieldErrors<keyof InvitationDraft>>({});
  readonly form = new Rx<FormStatus>(idleForm);
}

export abstract class InvitationActions {
  abstract edit(patch: Partial<InvitationDraft>): void;
  abstract accept(): Promise<void>;
  abstract register(): Promise<void>;
  abstract signOut(): Promise<void>;
}
