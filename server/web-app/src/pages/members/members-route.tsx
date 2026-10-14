import { useNavigate } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { MembersController } from "./members-controller";
import { MembersPage } from "./members-page";

export function MembersRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  return (
    <Subtree
      key="members"
      deps={[orgId]}
      controller={() =>
        new MembersController(
          pick(deps, "client", "session", "authHandler", "meRepository", "orgRepository"),
          { onLeft: () => navigate(paths.home, { replace: true }) },
          orgId,
        )
      }
    >
      <MembersPage />
    </Subtree>
  );
}
