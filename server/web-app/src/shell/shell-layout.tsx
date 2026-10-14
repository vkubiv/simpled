import type { Account, MyOrganization } from "@simpled/backend-client";
import type { ReactNode } from "react";
import { Link, NavLink } from "react-router-dom";
import { useObserver, useSubtree } from "subtree.js/react";
import { LoadFailed, Logo, RoleBadge, Spinner } from "../components/ui";
import { paths } from "../core/primitives/routes";
import { ShellActions, ShellState } from "./shell-model";

const navCls = ({ isActive }: { isActive: boolean }) =>
  `block rounded-btn px-3 py-2 text-sm font-medium transition-colors ${
    isActive ? "bg-accent-soft text-accent-strong" : "text-muted hover:bg-sunken hover:text-ink"
  }`;

/** Header, organization switcher and navigation around the organization's pages. */
export function ShellLayout({ children }: { children: ReactNode }) {
  const state = useSubtree(ShellState);
  const actions = useSubtree(ShellActions);
  const observer = useObserver();
  const view = observer.watch(state.view);
  const org = observer.watch(state.org);
  const failure = observer.watch(state.failure);
  const account = observer.watch(state.account);
  const organizations = observer.watch(state.organizations);
  const orgId = state.orgId;

  return (
    <div className="min-h-screen">
      <TopBar
        account={account}
        organizations={organizations}
        currentOrgId={orgId}
        currentName={org?.name ?? null}
        onSignOut={() => void actions.signOut()}
      />

      {view === "loading" && (
        <div className="flex justify-center p-16">
          <Spinner />
        </div>
      )}
      {view === "notFound" && (
        <div className="mx-auto max-w-md p-10 text-center">
          <h1 className="text-lg font-semibold">Organization not found</h1>
          <p className="mt-1 text-sm text-muted">It was deleted, or you are no longer a member.</p>
          <Link to={paths.home} className="btn btn-secondary mt-5">
            Go to my organizations
          </Link>
        </div>
      )}
      {view === "failed" && (
        <div className="mx-auto max-w-md p-10">
          <LoadFailed message={failure ?? ""} onRetry={() => void actions.retry()} />
        </div>
      )}
      {view === "ready" && org && (
        <div className="mx-auto flex max-w-6xl flex-col gap-6 px-4 py-6 md:flex-row md:px-6">
          <nav aria-label="Organization" className="md:w-48 md:shrink-0">
            <ul className="flex gap-1 overflow-x-auto md:flex-col">
              <li>
                <NavLink to={paths.org(orgId)} end className={navCls}>
                  Overview
                </NavLink>
              </li>
              <li>
                <NavLink to={paths.projects(orgId)} className={navCls}>
                  Projects
                </NavLink>
              </li>
              <li>
                <NavLink to={paths.teams(orgId)} className={navCls}>
                  Teams
                </NavLink>
              </li>
              <li>
                <NavLink to={paths.members(orgId)} className={navCls}>
                  Members
                </NavLink>
              </li>
              <li>
                <NavLink to={paths.settings(orgId)} className={navCls}>
                  Settings
                </NavLink>
              </li>
            </ul>
          </nav>
          <main className="min-w-0 flex-1">{children}</main>
        </div>
      )}
    </div>
  );
}

/** The bar every signed-in page shows: the switcher between the account's organizations, and the account. */
export function TopBar({
  account,
  organizations,
  currentOrgId,
  currentName,
  onSignOut,
}: {
  account: Account | null;
  organizations: readonly MyOrganization[];
  currentOrgId: number | null;
  currentName: string | null;
  onSignOut: () => void;
}) {
  const current = organizations.find((o) => o.id === currentOrgId);
  return (
    <header className="sticky top-0 z-10 border-b border-line bg-panel/90 backdrop-blur">
      <div className="mx-auto flex h-14 max-w-6xl items-center gap-2 px-4 sm:gap-3 md:px-6">
        <Link to={paths.home} aria-label="simpled home" className="shrink-0">
          <Logo compact />
        </Link>
        <span className="hidden text-line-strong sm:inline" aria-hidden="true">
          /
        </span>

        <details className="group relative min-w-0">
          <summary className="flex cursor-pointer list-none items-center gap-2 rounded-btn px-2 py-1.5 text-sm font-medium hover:bg-sunken">
            <span className="max-w-[7rem] truncate sm:max-w-[12rem]">
              {currentName ?? current?.name ?? "Organizations"}
            </span>
            {current && <RoleBadge role={current.role} />}
            <svg aria-hidden="true" viewBox="0 0 20 20" className="h-4 w-4 text-faint" fill="currentColor">
              <path d="M5.5 7.5 10 12l4.5-4.5" stroke="currentColor" strokeWidth="1.5" fill="none" />
            </svg>
          </summary>
          <div className="absolute left-0 mt-1 w-72 rounded-panel border border-line bg-panel p-1 shadow-lg">
            <p className="px-3 pb-1 pt-2 text-xs font-medium uppercase tracking-wide text-faint">Organizations</p>
            <ul>
              {organizations.map((o) => (
                <li key={o.id}>
                  <Link
                    to={paths.org(o.id)}
                    className={`flex items-center justify-between gap-2 rounded-btn px-3 py-2 text-sm hover:bg-sunken ${
                      o.id === currentOrgId ? "font-medium text-accent-strong" : ""
                    }`}
                  >
                    <span className="truncate">{o.name}</span>
                    <RoleBadge role={o.role} />
                  </Link>
                </li>
              ))}
            </ul>
            <div className="mt-1 border-t border-line pt-1">
              <Link to={paths.newOrg} className="block rounded-btn px-3 py-2 text-sm text-accent hover:bg-sunken">
                + New organization
              </Link>
            </div>
          </div>
        </details>

        <div className="ml-auto flex shrink-0 items-center gap-3">
          {account && (
            <span className="hidden text-right text-sm leading-tight sm:block">
              <span className="block font-medium">{account.name}</span>
              <span className="block text-xs text-faint">{account.email}</span>
            </span>
          )}
          <button type="button" className="btn btn-ghost btn-sm" onClick={onSignOut}>
            Sign out
          </button>
        </div>
      </div>
    </header>
  );
}
