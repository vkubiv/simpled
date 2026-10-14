import { useNavigate } from "react-router-dom";
import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { paths } from "../../core/primitives/routes";
import { OrgSettingsController } from "./org-settings-controller";
import { OrgSettingsPage } from "./org-settings-page";

export function OrgSettingsRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  const navigate = useNavigate();
  return (
    <Subtree
      key="org-settings"
      deps={[orgId]}
      controller={() =>
        new OrgSettingsController(
          pick(deps, "client", "session", "authHandler", "meRepository", "orgRepository"),
          { onGone: () => navigate(paths.home, { replace: true }) },
          orgId,
        )
      }
    >
      <OrgSettingsPage />
    </Subtree>
  );
}
