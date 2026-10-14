import { useNavigate } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { ProjectsController } from "./projects-controller";
import { ProjectsPage } from "./projects-page";

export function ProjectsRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  return (
    <Subtree
      key="projects"
      deps={[orgId]}
      controller={() =>
        new ProjectsController(
          pick(deps, "client", "authHandler", "orgRepository"),
          { onCreated: (projectId) => navigate(paths.project(orgId, projectId)) },
          orgId,
        )
      }
    >
      <ProjectsPage />
    </Subtree>
  );
}
