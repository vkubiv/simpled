import { Navigate, useNavigate } from "react-router-dom";
import { Subtree, useObserver, useSubtree } from "subtree.js/react";
import { pick } from "trunk.js";
import { AppState } from "../../app-controller";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { RegisterController } from "./register-controller";
import { RegisterPage } from "./register-page";

export function RegisterRoute({ deps }: { deps: AppDeps }) {
  const state = useSubtree(AppState);
  const observer = useObserver();
  const navigate = useNavigate();
  if (observer.watch(state.gate) === "signedIn") return <Navigate to={paths.home} replace />;
  return (
    <Subtree
      key="register"
      controller={() =>
        new RegisterController(pick(deps, "client", "session"), {
          onRegistered: (email) => navigate(paths.checkEmail(email)),
        })
      }
    >
      <RegisterPage />
    </Subtree>
  );
}
