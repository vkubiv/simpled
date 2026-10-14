import { Subtree } from "subtree.js/react";
import { pick } from "trunk.js";
import type { AppDeps } from "../../app-deps";
import { GitHubController } from "./github-controller";
import { GitHubPage } from "./github-page";

export function GitHubRoute({ deps, orgId }: { deps: AppDeps; orgId: number }) {
  return (
    <Subtree
      key="github"
      deps={[orgId]}
      controller={() => new GitHubController(pick(deps, "client", "authHandler", "orgRepository"), orgId)}
    >
      <GitHubPage />
    </Subtree>
  );
}
