import { useNavigate, useSearchParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { ResetPasswordController } from "./reset-password-controller";
import { ResetPasswordPage } from "./reset-password-page";

export function ResetPasswordRoute({ deps }: { deps: AppDeps }) {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const token = params.get("token") ?? "";
  return (
    <Subtree
      key="reset-password"
      deps={[token]}
      controller={() =>
        new ResetPasswordController(
          pick(deps, "client", "session"),
          { onReset: () => navigate("/sign-in?notice=reset", { replace: true }) },
          token,
        )
      }
    >
      <ResetPasswordPage />
    </Subtree>
  );
}
