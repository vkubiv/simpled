import { EmailNotConfirmed, fail, InvalidCredentials, ok } from "@simpled/backend-client";
import { describe, expect, it, type Mock, vi } from "vitest";
import { fakeClient, makeDeps, makeMe } from "../../testing/fakes";
import { SignInController } from "./sign-in-controller";

function setup(login: Mock = vi.fn(async () => ok(undefined))) {
  const deps = makeDeps(fakeClient({ auth: { login }, me: { get: async () => ok(makeMe()) } }));
  const controller = new SignInController(deps);
  return { controller, state: controller.state, deps, login };
}

describe("SignInController", () => {
  it("signs in and loads the account", async () => {
    const { controller, state, deps, login } = setup();
    controller.setEmail(" ann@example.com ");
    controller.setPassword("correct-horse-battery");
    await controller.submit();

    expect(login).toHaveBeenCalledWith(deps.session.context, "ann@example.com", "correct-horse-battery");
    expect(deps.session.stage).toBe("signedIn");
    expect(deps.meRepository.me?.account.email).toBe("ann@example.com");
    expect(state.busy.value).toBe(false);
  });

  it("checks the fields before calling, and again as they change", async () => {
    const { controller, state, login } = setup();
    controller.setEmail("not-an-email");
    await controller.submit();
    expect(login).not.toHaveBeenCalled();
    expect(state.fieldErrors.value).toEqual({
      email: "Enter a valid email address.",
      password: "Enter your password.",
    });

    controller.setPassword("x");
    expect(state.fieldErrors.value.password).toBeUndefined();
  });

  it("wrong credentials show the client's sentence", async () => {
    const { controller, state, deps } = setup(vi.fn(async () => fail(new InvalidCredentials())));
    controller.setEmail("ann@example.com");
    controller.setPassword("wrong");
    await controller.submit();
    expect(state.error.value).toBe("Wrong email or password.");
    expect(deps.session.stage).toBe("signedOut");
  });

  it("an unconfirmed email offers a new link instead of an error", async () => {
    const { controller, state } = setup(vi.fn(async () => fail(new EmailNotConfirmed())));
    controller.setEmail("ann@example.com");
    controller.setPassword("correct-horse-battery");
    await controller.submit();
    expect(state.unconfirmedEmail.value).toBe("ann@example.com");
    expect(state.error.value).toBeNull();
  });
});
