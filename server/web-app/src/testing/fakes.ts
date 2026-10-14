/**
 * Test doubles: a client built from the API groups this app calls (only the stubbed methods exist), deps around a
 * session in a given stage, and model factories with every field the Zod schemas require. No React.
 */
import {
  anonymous,
  type Deployment,
  type GitHubConnection,
  type Me,
  type Member,
  type MyOrganization,
  type Organization,
  type OrgRole,
  type Project,
  type Repository,
  type Team,
  type TeamSummary,
} from "@simpled/backend-client";
import { type AppDeps, createAppDeps } from "../app-deps";
import { AccountSession, type SessionStage } from "../core/auth/account-session";
import type { BackendApi } from "../core/ops/api";

export const NOW = new Date("2026-10-14T10:00:00Z");

export type FakeClientParts = { [K in keyof BackendApi]?: Partial<BackendApi[K]> };

export function fakeClient(parts: FakeClientParts): BackendApi {
  return {
    auth: parts.auth ?? {},
    me: parts.me ?? {},
    orgs: parts.orgs ?? {},
    invitations: parts.invitations ?? {},
    teams: parts.teams ?? {},
    projects: parts.projects ?? {},
    gitHubConnections: parts.gitHubConnections ?? {},
    repositories: parts.repositories ?? {},
  } as unknown as BackendApi;
}

export function makeDeps(client: BackendApi, stage: SessionStage = "signedOut"): AppDeps {
  const session = new AccountSession(anonymous);
  session.setStage(stage);
  return createAppDeps({ client, session });
}

/** Signed in as ann@example.com, in `org` with its role, the organization shown by the shell. */
export function signedInDeps(client: BackendApi, org: Organization = makeOrg()): AppDeps {
  const deps = makeDeps(client, "signedIn");
  deps.meRepository.set({
    kind: "loaded",
    me: makeMe([{ id: org.id, name: org.name, slug: org.slug, role: org.role }]),
  });
  deps.orgRepository.set({ kind: "loaded", org });
  return deps;
}

/** Let every pending microtask and zero-delay timer run. */
export const flush = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));

export function makeMe(organizations: MyOrganization[] = []): Me {
  return { account: { id: "acc-ann", email: "ann@example.com", name: "Ann" }, organizations };
}

const PERMISSIONS: Record<OrgRole, Organization["permissions"]> = {
  owner: {
    canManageOrganization: true,
    canManageMembers: true,
    canCreateTeam: true,
    canCreateProject: true,
    canManageGitHubConnections: true,
    assignableRoles: ["owner", "admin", "manager", "member"],
  },
  admin: {
    canManageOrganization: false,
    canManageMembers: true,
    canCreateTeam: true,
    canCreateProject: true,
    canManageGitHubConnections: true,
    assignableRoles: ["admin", "manager", "member"],
  },
  manager: {
    canManageOrganization: false,
    canManageMembers: false,
    canCreateTeam: true,
    canCreateProject: true,
    canManageGitHubConnections: false,
    assignableRoles: [],
  },
  member: {
    canManageOrganization: false,
    canManageMembers: false,
    canCreateTeam: false,
    canCreateProject: false,
    canManageGitHubConnections: false,
    assignableRoles: [],
  },
};

export function makeOrg(role: OrgRole = "owner", o: Partial<Organization> = {}): Organization {
  return { id: 1, name: "Acme", slug: "acme", createdAt: NOW, role, permissions: PERMISSIONS[role], ...o };
}

export function makeMember(o: Partial<Member> = {}): Member {
  return {
    accountId: "acc-bob",
    email: "bob@example.com",
    name: "Bob",
    role: "member",
    joinedAt: NOW,
    isMe: false,
    canChangeRole: true,
    canRemove: true,
    ...o,
  };
}

export function makeTeamSummary(o: Partial<TeamSummary> = {}): TeamSummary {
  return { id: 10, name: "Backend", memberCount: 1, projectCount: 0, isMember: false, canManage: true, ...o };
}

export function makeTeam(o: Partial<Team> = {}): Team {
  return {
    id: 10,
    organizationId: 1,
    name: "Backend",
    createdAt: NOW,
    members: [],
    projects: [],
    isMember: false,
    canManage: true,
    ...o,
  };
}

export function makeProject(o: Partial<Project> = {}): Project {
  return {
    id: 100,
    organizationId: 1,
    name: "Web shop",
    slug: "web-shop",
    createdAt: NOW,
    teams: [],
    isMember: false,
    canManage: true,
    ...o,
  };
}

export function makeConnection(o: Partial<GitHubConnection> = {}): GitHubConnection {
  return { id: 200, name: "Deploy bot", gitHubLogin: "octocat", createdAt: NOW, repositoryCount: 0, ...o };
}

export function makeDeployment(o: Partial<Deployment> = {}): Deployment {
  return {
    id: 400,
    name: "prod",
    applicationName: "shop",
    versionConstraint: "^1.0.0",
    url: "https://shop.example.com",
    repoVersion: "1.1.0",
    status: "up",
    statusSince: NOW,
    liveVersion: "1.0.0",
    liveDeployedAt: NOW,
    checkedAt: NOW,
    checkError: null,
    ...o,
  };
}

export function makeRepository(o: Partial<Repository> = {}): Repository {
  return {
    id: 300,
    projectId: 100,
    connectionId: 200,
    owner: "acme",
    name: "shop",
    defaultBranch: "main",
    createdAt: NOW,
    syncedAt: NOW,
    syncError: null,
    environments: [{ id: 350, name: "prod", path: "deployment/prod/envspec.yaml", deployments: [makeDeployment()] }],
    canManage: true,
    ...o,
  };
}
