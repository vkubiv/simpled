import { useSearchParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { CheckEmailController } from "./check-email-controller";
import { CheckEmailPage } from "./check-email-page";

export function CheckEmailRoute({ deps }: { deps: AppDeps }) {
  const [params] = useSearchParams();
  const email = params.get("email") ?? "";
  return (
    <Subtree
      key="check-email"
      deps={[email]}
      controller={() => new CheckEmailController(pick(deps, "client", "session"), email)}
    >
      <CheckEmailPage />
    </Subtree>
  );
}
