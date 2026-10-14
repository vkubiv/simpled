import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { OrgOverviewController } from "./org-overview-controller";
import { OrgOverviewPage } from "./org-overview-page";

export function OrgOverviewRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  return (
    <Subtree
      key="org-overview"
      deps={[orgId]}
      controller={() => new OrgOverviewController(pick(deps, "client", "authHandler", "orgRepository"), orgId)}
    >
      <OrgOverviewPage />
    </Subtree>
  );
}
