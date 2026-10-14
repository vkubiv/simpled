import { useLocation, useNavigate, useSearchParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { InvitationController } from "./invitation-controller";
import { InvitationPage } from "./invitation-page";

export function InvitationRoute({ deps }: { deps: AppDeps }) {
  const [params] = useSearchParams();
  const location = useLocation();
  const navigate = useNavigate();
  const token = params.get("token") ?? "";
  return (
    <Subtree
      key="invitation"
      deps={[token]}
      controller={() =>
        new InvitationController(
          pick(deps, "client", "session", "authHandler", "meRepository"),
          { onJoined: (orgId) => navigate(orgId === null ? paths.home : paths.org(orgId), { replace: true }) },
          token,
        )
      }
    >
      <InvitationPage returnTo={location.pathname + location.search} />
    </Subtree>
  );
}
