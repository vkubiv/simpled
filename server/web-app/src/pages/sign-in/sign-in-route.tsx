import { Navigate, useSearchParams } from "react-router-dom";
import { Subtree, useObserver, useSubtree } from "subtree.js/react";
import { pick } from "trunk.js";
import { AppState } from "../../app-controller";
import type { AppDeps } from "../../app-deps";
import { safeNext } from "../../core/primitives/forms";
import { SignInController } from "./sign-in-controller";
import { SignInPage } from "./sign-in-page";

const NOTICES: Record<string, string> = {
  reset: "Your password was changed. Sign in with the new one.",
};

/** Once signed in, goes on to `?next=` (where the gate sent the person from), or home. */
export function SignInRoute({ deps }: { deps: AppDeps }) {
  const state = useSubtree(AppState);
  const observer = useObserver();
  const [params] = useSearchParams();
  if (observer.watch(state.gate) === "signedIn") return <Navigate to={safeNext(params.get("next"))} replace />;
  const notice = NOTICES[params.get("notice") ?? ""] ?? null;
  return (
    <Subtree
      key="sign-in"
      controller={() => new SignInController(pick(deps, "client", "session", "meRepository"), notice)}
    >
      <SignInPage />
    </Subtree>
  );
}
