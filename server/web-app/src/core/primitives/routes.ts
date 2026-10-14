/** Paths in one place: links in views and routing callbacks in route components agree on them. */
export const paths = {
  home: "/",
  signIn: (next?: string) => (next && next !== "/" ? `/sign-in?next=${encodeURIComponent(next)}` : "/sign-in"),
  register: "/register",
  checkEmail: (email: string) => `/check-email?email=${encodeURIComponent(email)}`,
  forgotPassword: "/forgot-password",
  newOrg: "/orgs/new",
  org: (orgId: number) => `/orgs/${orgId}`,
  members: (orgId: number) => `/orgs/${orgId}/members`,
  teams: (orgId: number) => `/orgs/${orgId}/teams`,
  team: (orgId: number, teamId: number) => `/orgs/${orgId}/teams/${teamId}`,
  projects: (orgId: number) => `/orgs/${orgId}/projects`,
  project: (orgId: number, projectId: number) => `/orgs/${orgId}/projects/${projectId}`,
  settings: (orgId: number) => `/orgs/${orgId}/settings`,
  github: (orgId: number) => `/orgs/${orgId}/github`,
};
