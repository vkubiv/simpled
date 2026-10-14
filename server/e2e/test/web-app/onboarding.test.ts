import { AppController } from "simpled-web-app/src/app-controller";
import { requestPasswordReset } from "simpled-web-app/src/core/ops/auth-ops";
import { CheckEmailController } from "simpled-web-app/src/pages/check-email/check-email-controller";
import { ConfirmEmailController } from "simpled-web-app/src/pages/confirm-email/confirm-email-controller";
import { RegisterController } from "simpled-web-app/src/pages/register/register-controller";
import { ResetPasswordController } from "simpled-web-app/src/pages/reset-password/reset-password-controller";
import { SignInController } from "simpled-web-app/src/pages/sign-in/sign-in-controller";
import { ShellController } from "simpled-web-app/src/shell/shell-controller";
import { afterEach, describe, expect, test, vi } from "vitest";
import { lastToken } from "../utils/mailbox";
import { PASSWORD, Sugar } from "../utils/sugar";
import { WebApp, waitFor } from "../utils/web-app";

const apps: WebApp[] = [];
const newApp = () => {
  const app = new WebApp();
  apps.push(app);
  return app;
};
afterEach(() => {
  for (const app of apps.splice(0)) app.dispose();
});

describe("web app: onboarding", () => {
  test("register, confirm through the emailed link, and land as owner of the first organization", async () => {
    const sugar = new Sugar();
    const app = newApp();
    const email = sugar.email("ann");
    const root = app.own(new AppController(app.deps));
    await waitFor(root.state.gate, (g) => g === "signedOut", "the gate");

    const registered = { onRegistered: vi.fn() };
    const register = app.own(new RegisterController(app.deps, registered));
    register.edit({ name: "Ann", email, password: PASSWORD, organizationName: sugar.orgName("Acme") });
    await register.submit();
    expect(registered.onRegistered).toHaveBeenCalledWith(email);

    const signIn = app.own(new SignInController(app.deps));
    signIn.setEmail(email);
    signIn.setPassword(PASSWORD);
    await signIn.submit();
    expect(signIn.state.unconfirmedEmail.value).toBe(email);

    const checkEmail = app.own(new CheckEmailController(app.deps, email));
    await checkEmail.resend();
    expect(checkEmail.state.resend.value.done).toContain("new link");

    const confirmed = { onConfirmed: vi.fn() };
    const confirm = app.own(new ConfirmEmailController(app.deps, confirmed, await lastToken(email, "confirm-email")));
    await confirm.confirm();
    expect(confirmed.onConfirmed).toHaveBeenCalled();
    expect(root.state.gate.value).toBe("signedIn");

    const orgId = app.deps.meRepository.me?.organizations[0]?.id;
    expect(root.state.home.value).toBe(`/orgs/${orgId}`);
    const shell = app.own(new ShellController(app.deps, orgId ?? 0));
    await waitFor(shell.state.view, (v) => v === "ready", "the shell");
    expect(shell.state.org.value).toMatchObject({ name: sugar.orgName("Acme"), role: "owner" });
    expect(shell.state.org.value?.permissions.canManageOrganization).toBe(true);

    await root.signOut();
    expect(root.state.gate.value).toBe("signedOut");
    expect(app.deps.meRepository.me).toBeNull();
  });

  test("a used confirmation link and a wrong password say so", async () => {
    const sugar = new Sugar();
    await sugar.register("ann");
    const email = sugar.email("ann");
    const app = newApp();

    const confirm = app.own(
      new ConfirmEmailController(app.deps, { onConfirmed: vi.fn() }, await lastToken(email, "confirm-email")),
    );
    await confirm.confirm();
    expect(confirm.state.error.value).toBe("The link is invalid or has expired.");

    const signIn = app.own(new SignInController(app.deps));
    signIn.setEmail(email);
    signIn.setPassword("not-the-password");
    await signIn.submit();
    expect(signIn.state.error.value).toBe("Wrong email or password.");
    expect(app.session.stage).not.toBe("signedIn");
  });

  test("a reset password signs in with the new one only", async () => {
    const sugar = new Sugar();
    await sugar.register("ann");
    const email = sugar.email("ann");
    const app = newApp();

    await requestPasswordReset({ ...app.deps, email });
    const reset = { onReset: vi.fn() };
    const page = app.own(new ResetPasswordController(app.deps, reset, await lastToken(email, "reset-password")));
    page.setPassword("short");
    await page.submit();
    expect(page.state.passwordError.value).toBe("Use at least 10 characters.");
    page.setPassword("a-brand-new-password");
    await page.submit();
    expect(reset.onReset).toHaveBeenCalled();

    const signIn = app.own(new SignInController(app.deps));
    signIn.setEmail(email);
    signIn.setPassword("a-brand-new-password");
    await signIn.submit();
    expect(app.session.stage).toBe("signedIn");
  });
});
