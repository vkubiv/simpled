import {
  AccountAlreadyExists,
  ApiNotAuthorized,
  anonymous,
  EmailNotConfirmed,
  InvalidCredentials,
  InvalidFormField,
  InvalidLink,
  withToken,
} from "@simpled/backend-client";
import { describe, expect, test } from "vitest";
import { expectError, expectSuccess } from "./utils/expect";
import { lastToken } from "./utils/mailbox";
import { PASSWORD, Sugar } from "./utils/sugar";

describe("auth", () => {
  test("registration needs the mailed confirmation, which signs in as owner of the first organization", async () => {
    const sugar = new Sugar();
    const { auth, me } = sugar.client;
    const ctx = sugar.newContext();
    const email = sugar.email("ann");

    expectSuccess(
      await auth.register(ctx, { email, password: PASSWORD, name: "Ann", organizationName: sugar.orgName("Acme") }),
    );
    expectError(await auth.login(ctx, email, PASSWORD), EmailNotConfirmed);

    const token = await lastToken(email, "confirm-email");
    expectSuccess(await auth.confirmEmail(ctx, token));
    expectError(await auth.confirmEmail(sugar.newContext(), token), InvalidLink);

    const signedIn = expectSuccess(await me.get(ctx));
    expect(signedIn.account).toMatchObject({ email, name: "Ann" });
    expect(signedIn.organizations).toEqual([expect.objectContaining({ name: sugar.orgName("Acme"), role: "owner" })]);
  });

  test("registration is refused for a taken email or an invalid field", async () => {
    const sugar = new Sugar();
    await sugar.register("ann");
    const registration = { email: sugar.email("ann"), password: PASSWORD, name: "Ann", organizationName: "X" };

    expectError(await sugar.client.auth.register(anonymous, registration), AccountAlreadyExists);
    const short = expectError(
      await sugar.client.auth.register(anonymous, { ...registration, email: sugar.email("bob"), password: "short" }),
      InvalidFormField,
    );
    expect(short.fieldName).toBe("password");
  });

  test("a resent confirmation replaces the earlier link", async () => {
    const sugar = new Sugar();
    const { auth } = sugar.client;
    const email = sugar.email("ann");
    expectSuccess(await auth.register(anonymous, { email, password: PASSWORD, name: "Ann", organizationName: "Acme" }));
    const first = await lastToken(email, "confirm-email");

    expectSuccess(await auth.resendConfirmation(anonymous, email));
    const second = await lastToken(email, "confirm-email");

    expect(second).not.toBe(first);
    expectError(await auth.confirmEmail(sugar.newContext(), first), InvalidLink);
    expectSuccess(await auth.confirmEmail(sugar.newContext(), second));
  });

  test("login with a cookie or a token, wrong password, logout", async () => {
    const sugar = new Sugar();
    const { auth, me } = sugar.client;
    await sugar.register("ann");
    const email = sugar.email("ann");

    expectError(await auth.login(sugar.newContext(), email, "wrong-password"), InvalidCredentials);

    const token = expectSuccess(await auth.loginWithToken(anonymous, email, PASSWORD)).token;
    expect(expectSuccess(await me.get(withToken(token))).account.email).toBe(email);

    const ctx = sugar.newContext();
    expectSuccess(await auth.login(ctx, email, PASSWORD));
    expectSuccess(await me.get(ctx));
    expectSuccess(await auth.logout(ctx));
    expectError(await me.get(ctx), ApiNotAuthorized);
    expectError(await me.get(anonymous), ApiNotAuthorized);
  });

  test("password reset signs out every session", async () => {
    const sugar = new Sugar();
    const { auth, me } = sugar.client;
    await sugar.register("ann");
    const email = sugar.email("ann");

    expectSuccess(await auth.requestPasswordReset(anonymous, email));
    expectSuccess(await auth.requestPasswordReset(anonymous, sugar.email("nobody")));
    const token = await lastToken(email, "reset-password");

    expectError(await auth.resetPassword(anonymous, token, "short"), InvalidFormField);
    expectSuccess(await auth.resetPassword(anonymous, token, "a-brand-new-password"));
    expectError(await auth.resetPassword(anonymous, token, "a-brand-new-password"), InvalidLink);

    expectError(await me.get(sugar.ctx("ann")), ApiNotAuthorized);
    expectError(await auth.login(sugar.newContext(), email, PASSWORD), InvalidCredentials);
    expectSuccess(await auth.login(sugar.newContext(), email, "a-brand-new-password"));
  });
});
