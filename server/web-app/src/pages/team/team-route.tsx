import { useNavigate, useParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { TeamController } from "./team-controller";
import { TeamPage } from "./team-page";

export function TeamRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  const teamId = Number(useParams<{ teamId: string }>().teamId);
  return (
    <Subtree
      key="team"
      deps={[orgId, teamId]}
      controller={() =>
        new TeamController(
          pick(deps, "client", "authHandler", "orgRepository"),
          { onDeleted: () => navigate(paths.teams(orgId), { replace: true }) },
          orgId,
          teamId,
        )
      }
    >
      <TeamPage />
    </Subtree>
  );
}
