import {
  AccountAlreadyExists,
  AlreadyMember,
  anonymous,
  InvalidFormField,
  InvalidLink,
  InvitationForAnotherEmail,
  InvitationNotFound,
  RoleForbidden,
} from "@simpled/backend-client";
import { describe, expect, test } from "vitest";
import { expectError, expectSuccess } from "./utils/expect";
import { lastToken, mailsTo } from "./utils/mailbox";
import { PASSWORD, Sugar } from "./utils/sugar";

describe("invitations", () => {
  test("a new person registers through the link, confirmed and signed in, with the invited role", async () => {
    const sugar = new Sugar();
    const { invitations, me } = sugar.client;
    const orgId = await sugar.org("ann");
    const email = sugar.email("bob");

    const invitation = expectSuccess(await invitations.invite(sugar.ctx("ann"), orgId, email, "manager"));
    expect(invitation).toMatchObject({ email, role: "manager", invitedBy: "Ann", expired: false });
    const [mail] = await mailsTo(email);
    expect(mail.subject).toBe(`Ann invited you to ${sugar.orgName("Acme")} on simpled`);

    const token = await lastToken(email, "invitation");
    const preview = expectSuccess(await invitations.preview(anonymous, token));
    expect(preview).toEqual({
      organizationName: sugar.orgName("Acme"),
      role: "manager",
      email,
      invitedBy: "Ann",
      accountExists: false,
    });

    const ctx = sugar.newContext();
    expectError(await invitations.register(ctx, token, "Bob", "short"), InvalidFormField);
    expectSuccess(await invitations.register(ctx, token, "Bob", PASSWORD));
    expect(expectSuccess(await me.get(ctx)).organizations).toEqual([
      expect.objectContaining({ id: orgId, role: "manager" }),
    ]);

    expectError(await invitations.preview(anonymous, token), InvalidLink);
    expectSuccess(await sugar.client.auth.login(sugar.newContext(), email, PASSWORD));
  });

  test("an existing account accepts while signed in with the invited email", async () => {
    const sugar = new Sugar();
    const { invitations } = sugar.client;
    const orgId = await sugar.org("ann");
    await sugar.register("bob", "Bob Co");
    await sugar.register("eve", "Eve Co");

    expectSuccess(await invitations.invite(sugar.ctx("ann"), orgId, sugar.email("bob"), "member"));
    const token = await lastToken(sugar.email("bob"), "invitation");
    expect(expectSuccess(await invitations.preview(anonymous, token)).accountExists).toBe(true);
    expectError(await invitations.register(sugar.newContext(), token, "Bob", PASSWORD), AccountAlreadyExists);

    expectError(await invitations.accept(sugar.ctx("eve"), token), InvitationForAnotherEmail);
    const joined = expectSuccess(await invitations.accept(sugar.ctx("bob"), token));
    expect(joined).toMatchObject({ id: orgId, role: "member" });
    expectError(await invitations.accept(sugar.ctx("bob"), token), InvalidLink);

    expectError(await invitations.invite(sugar.ctx("ann"), orgId, sugar.email("bob"), "admin"), AlreadyMember);
  });

  test("who may invite whom", async () => {
    const sugar = new Sugar();
    const { invitations } = sugar.client;
    const orgId = await sugar.org("ann", { bob: "admin", cid: "manager" });

    expectError(await invitations.invite(sugar.ctx("cid"), orgId, sugar.email("x"), "member"), RoleForbidden);
    expectError(await invitations.invite(sugar.ctx("bob"), orgId, sugar.email("x"), "owner"), RoleForbidden);
    expectError(await invitations.list(sugar.ctx("cid"), orgId), RoleForbidden);
    const invalid = expectError(
      await invitations.invite(sugar.ctx("bob"), orgId, "not-an-email", "member"),
      InvalidFormField,
    );
    expect(invalid.fieldName).toBe("email");

    expectSuccess(await invitations.invite(sugar.ctx("bob"), orgId, sugar.email("x"), "admin"));
  });

  test("inviting again replaces the link; revoking ends it", async () => {
    const sugar = new Sugar();
    const { invitations } = sugar.client;
    const orgId = await sugar.org("ann");
    const email = sugar.email("bob");

    expectSuccess(await invitations.invite(sugar.ctx("ann"), orgId, email, "member"));
    const first = await lastToken(email, "invitation");
    const second = expectSuccess(await invitations.invite(sugar.ctx("ann"), orgId, email, "admin"));
    const secondToken = await lastToken(email, "invitation");

    expectError(await invitations.preview(anonymous, first), InvalidLink);
    expect(expectSuccess(await invitations.preview(anonymous, secondToken)).role).toBe("admin");
    expect(expectSuccess(await invitations.list(sugar.ctx("ann"), orgId)).map((i) => i.id)).toEqual([second.id]);

    expectSuccess(await invitations.revoke(sugar.ctx("ann"), orgId, second.id));
    expectError(await invitations.revoke(sugar.ctx("ann"), orgId, second.id), InvitationNotFound);
    expectError(await invitations.preview(anonymous, secondToken), InvalidLink);
    expect(expectSuccess(await invitations.list(sugar.ctx("ann"), orgId))).toEqual([]);
  });
});
