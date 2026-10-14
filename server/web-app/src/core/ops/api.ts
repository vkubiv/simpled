import type { MainClient } from "@simpled/backend-client";

/** The API groups this app calls; `MainClient` satisfies it, and tests fake only these. */
export type BackendApi = Pick<
  MainClient,
  "auth" | "me" | "orgs" | "invitations" | "teams" | "projects" | "gitHubConnections" | "repositories"
>;
