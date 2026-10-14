import { useNavigate } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { TeamsController } from "./teams-controller";
import { TeamsPage } from "./teams-page";

export function TeamsRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  return (
    <Subtree
      key="teams"
      deps={[orgId]}
      controller={() =>
        new TeamsController(
          pick(deps, "client", "authHandler", "orgRepository"),
          { onCreated: (teamId) => navigate(paths.team(orgId, teamId)) },
          orgId,
        )
      }
    >
      <TeamsPage />
    </Subtree>
  );
}
