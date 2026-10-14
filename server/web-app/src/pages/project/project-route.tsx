import { useNavigate, useParams } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { DeploymentsController } from "../project-deployments/deployments-controller";
import { DeploymentsSection } from "../project-deployments/deployments-section";
import { ProjectController } from "./project-controller";
import { ProjectPage } from "./project-page";

export function ProjectRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  const projectId = Number(useParams<{ projectId: string }>().projectId);
  return (
    <Subtree
      key="project"
      deps={[orgId, projectId]}
      controller={() =>
        new ProjectController(
          pick(deps, "client", "authHandler", "orgRepository"),
          { onDeleted: () => navigate(paths.projects(orgId), { replace: true }) },
          orgId,
          projectId,
        )
      }
    >
      <ProjectPage
        deployments={
          <Subtree
            key="project-deployments"
            deps={[orgId, projectId]}
            controller={() =>
              new DeploymentsController(pick(deps, "client", "authHandler", "orgRepository"), orgId, projectId)
            }
          >
            <DeploymentsSection />
          </Subtree>
        }
      />
    </Subtree>
  );
}
