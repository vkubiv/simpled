import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { ForgotPasswordController } from "./forgot-password-controller";
import { ForgotPasswordPage } from "./forgot-password-page";

export function ForgotPasswordRoute({ deps }: { deps: AppDeps }) {
  return (
    <Subtree key="forgot-password" controller={() => new ForgotPasswordController(pick(deps, "client", "session"))}>
      <ForgotPasswordPage />
    </Subtree>
  );
}
