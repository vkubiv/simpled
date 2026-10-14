import { useNavigate } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { NewOrgController } from "./new-org-controller";
import { NewOrgPage } from "./new-org-page";

export function NewOrgRoute({ deps }: { deps: AppDeps }) {
  const navigate = useNavigate();
  return (
    <Subtree
      key="new-org"
      controller={() =>
        new NewOrgController(pick(deps, "client", "session", "authHandler", "meRepository"), {
          onCreated: (orgId) => navigate(paths.org(orgId)),
        })
      }
    >
      <NewOrgPage />
    </Subtree>
  );
}
