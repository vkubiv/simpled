import { useNavigate, useSearchParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { ConfirmEmailController } from "./confirm-email-controller";
import { ConfirmEmailPage } from "./confirm-email-page";

export function ConfirmEmailRoute({ deps }: { deps: AppDeps }) {
  const [params] = useSearchParams();
  const navigate = useNavigate();
  const token = params.get("token") ?? "";
  return (
    <Subtree
      key="confirm-email"
      deps={[token]}
      controller={() =>
        new ConfirmEmailController(
          pick(deps, "client", "session", "meRepository"),
          { onConfirmed: () => navigate(paths.home, { replace: true }) },
          token,
        )
      }
    >
      <ConfirmEmailPage />
    </Subtree>
  );
}
