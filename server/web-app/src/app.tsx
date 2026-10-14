import type { ReactNode } from "react";
import { Navigate, Route, Routes, useLocation, useParams } from "react-router-dom";
import { Subtree, useObserver, useSubtree } from "subtree.js/react";
import { pick } from "trunk.js";
import { AppActions, AppController, AppState } from "./app-controller";
import type { AppDeps } from "./app-deps";
import { AuthCard, FullScreenSpinner } from "./components/ui";
import { paths } from "./core/primitives/routes";
import { CheckEmailRoute } from "./pages/check-email/check-email-route";
import { ConfirmEmailRoute } from "./pages/confirm-email/confirm-email-route";
import { ForgotPasswordRoute } from "./pages/forgot-password/forgot-password-route";
import { GitHubRoute } from "./pages/github/github-route";
import { InvitationRoute } from "./pages/invitation/invitation-route";
import { MembersRoute } from "./pages/members/members-route";
import { NewOrgRoute } from "./pages/new-org/new-org-route";
import { OrgOverviewRoute } from "./pages/org-overview/org-overview-route";
import { OrgSettingsRoute } from "./pages/org-settings/org-settings-route";
import { ProjectRoute } from "./pages/project/project-route";
import { ProjectsRoute } from "./pages/projects/projects-route";
import { RegisterRoute } from "./pages/register/register-route";
import { ResetPasswordRoute } from "./pages/reset-password/reset-password-route";
import { SignInRoute } from "./pages/sign-in/sign-in-route";
import { TeamRoute } from "./pages/team/team-route";
import { TeamsRoute } from "./pages/teams/teams-route";
import { ShellController } from "./shell/shell-controller";
import { ShellLayout } from "./shell/shell-layout";

/** Render inside a router (`BrowserRouter` in `main.tsx`). */
export function App({ deps }: { deps: AppDeps }) {
  return (
    <Subtree controller={() => new AppController(pick(deps, "client", "session", "meRepository", "orgRepository"))}>
      <Routes>
        {/* Signed-out pages; the emailed links (confirm, reset, invitation) land on these. */}
        <Route path="/sign-in" element={<SignInRoute deps={deps} />} />
        <Route path="/register" element={<RegisterRoute deps={deps} />} />
        <Route path="/check-email" element={<CheckEmailRoute deps={deps} />} />
        <Route path="/confirm-email" element={<ConfirmEmailRoute deps={deps} />} />
        <Route path="/forgot-password" element={<ForgotPasswordRoute deps={deps} />} />
        <Route path="/reset-password" element={<ResetPasswordRoute deps={deps} />} />
        <Route
          path="/invitation"
          element={
            <GateReady>
              <InvitationRoute deps={deps} />
            </GateReady>
          }
        />
        <Route path="*" element={<Protected deps={deps} />} />
      </Routes>
    </Subtree>
  );
}

/** Waits for the first `/Me`, so a page that differs signed in and out does not flash the wrong one. */
function GateReady({ children }: { children: ReactNode }) {
  const state = useSubtree(AppState);
  const observer = useObserver();
  return observer.watch(state.gate) === "loading" ? <FullScreenSpinner /> : children;
}

/** Stands between a visitor and the signed-in area. */
function Protected({ deps }: { deps: AppDeps }) {
  const state = useSubtree(AppState);
  const actions = useSubtree(AppActions);
  const observer = useObserver();
  const location = useLocation();
  const failure = observer.watch(state.failure);
  const home = observer.watch(state.home);

  switch (observer.watch(state.gate)) {
    case "loading":
      return <FullScreenSpinner />;
    case "failed":
      return (
        <AuthCard title="simpled is not answering" subtitle={failure ?? undefined}>
          <button type="button" className="btn btn-primary w-full" onClick={actions.retry}>
            Try again
          </button>
        </AuthCard>
      );
    case "signedOut":
      return <Navigate to={paths.signIn(location.pathname + location.search)} replace />;
    case "signedIn":
      return (
        <Routes>
          <Route path="/" element={<Navigate to={home} replace />} />
          <Route path="/orgs/new" element={<NewOrgRoute deps={deps} />} />
          <Route path="/orgs/:orgId/*" element={<OrgArea deps={deps} />} />
          <Route path="*" element={<Navigate to={home} replace />} />
        </Routes>
      );
  }
}

/** One organization's pages inside the shell; the shell is rebuilt when the URL's organization changes. */
function OrgArea({ deps }: { deps: AppDeps }) {
  const orgId = Number(useParams<{ orgId: string }>().orgId);
  if (!Number.isInteger(orgId)) return <Navigate to={paths.home} replace />;
  return (
    <Subtree
      key="shell"
      deps={[orgId]}
      controller={() =>
        new ShellController(pick(deps, "client", "session", "authHandler", "meRepository", "orgRepository"), orgId)
      }
    >
      <ShellLayout>
        <Routes>
          <Route index element={<OrgOverviewRoute deps={deps} orgId={orgId} />} />
          <Route path="members" element={<MembersRoute deps={deps} orgId={orgId} />} />
          <Route path="teams" element={<TeamsRoute deps={deps} orgId={orgId} />} />
          <Route path="teams/:teamId" element={<TeamRoute deps={deps} orgId={orgId} />} />
          <Route path="projects" element={<ProjectsRoute deps={deps} orgId={orgId} />} />
          <Route path="projects/:projectId" element={<ProjectRoute deps={deps} orgId={orgId} />} />
          <Route path="github" element={<GitHubRoute deps={deps} orgId={orgId} />} />
          <Route path="settings" element={<OrgSettingsRoute deps={deps} orgId={orgId} />} />
          <Route path="*" element={<Navigate to={paths.org(orgId)} replace />} />
        </Routes>
      </ShellLayout>
    </Subtree>
  );
}
