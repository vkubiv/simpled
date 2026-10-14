import { AccountAlreadyExists, fail, InvalidFormField, ok, RateLimited } from "@simpled/backend-client";
import { describe, expect, it, type Mock, vi } from "vitest";
import { fakeClient, makeDeps } from "../../testing/fakes";
import { RegisterController } from "./register-controller";

const VALID = { name: "Ann", email: "ann@example.com", password: "correct-horse-battery", organizationName: "Acme" };

function setup(register: Mock = vi.fn(async () => ok(undefined))) {
  const routing = { onRegistered: vi.fn() };
  const controller = new RegisterController(makeDeps(fakeClient({ auth: { register } })), routing);
  return { controller, state: controller.state, routing, register };
}

describe("RegisterController", () => {
  it("registers and sends the person to their inbox", async () => {
    const { controller, routing, register } = setup();
    controller.edit({ ...VALID, email: " ann@example.com " });
    await controller.submit();
    expect(register).toHaveBeenCalledWith(expect.anything(), VALID);
    expect(routing.onRegistered).toHaveBeenCalledWith("ann@example.com");
  });

  it("checks every field first, including a password that is the email", async () => {
    const { controller, state, register } = setup();
    controller.edit({ email: "ann@example.com", password: "ann@example.com" });
    await controller.submit();
    expect(register).not.toHaveBeenCalled();
    expect(state.fieldErrors.value).toEqual({
      name: "Enter your name.",
      email: undefined,
      password: "Don't use your email as password.",
      organizationName: "Enter an organization name.",
    });
  });

  it("a taken email and the backend's field errors land on their fields", async () => {
    const { controller, state, routing } = setup(
      vi.fn(async () =>
        fail(new AccountAlreadyExists(), new InvalidFormField("organizationName", "max length is 100")),
      ),
    );
    controller.edit(VALID);
    await controller.submit();
    expect(routing.onRegistered).not.toHaveBeenCalled();
    expect(state.fieldErrors.value).toEqual({
      email: "An account with this email already exists.",
      organizationName: "max length is 100",
    });
    expect(state.error.value).toBeNull();
  });

  it("other refusals show above the form", async () => {
    const { controller, state } = setup(vi.fn(async () => fail(new RateLimited("/Auth/Register"))));
    controller.edit(VALID);
    await controller.submit();
    expect(state.error.value).toBe("Too many attempts. Wait a minute and try again.");
  });
});
