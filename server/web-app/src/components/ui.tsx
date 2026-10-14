import type { OrgRole } from "@simpled/backend-client";
import type { ReactNode } from "react";
import { ROLE_LABELS } from "../core/primitives/roles";

/** Presentational pieces only: props in, callbacks out. */

export function Spinner({ className = "" }: { className?: string }) {
  return (
    <div
      role="status"
      aria-label="Loading"
      className={`h-5 w-5 animate-spin rounded-full border-2 border-accent border-t-transparent ${className}`}
    />
  );
}

export function FullScreenSpinner() {
  return (
    <div className="flex min-h-screen items-center justify-center">
      <Spinner />
    </div>
  );
}

/** `compact`: the mark alone on narrow screens, where the header has no room for the word. */
export function Logo({ compact = false }: { compact?: boolean }) {
  return (
    <span className="inline-flex items-center gap-2 text-[15px] font-semibold tracking-tight">
      <img src="/favicon.svg" alt="" className="h-6 w-6" />
      <span className={compact ? "hidden sm:inline" : undefined}>simpled</span>
    </span>
  );
}

/** The centred card the signed-out pages share. */
export function AuthCard({ title, subtitle, children }: { title: string; subtitle?: ReactNode; children: ReactNode }) {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center px-4 py-10">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex justify-center">
          <Logo />
        </div>
        <div className="panel p-6 shadow-sm sm:p-8">
          <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
          {subtitle && <p className="mt-1.5 text-sm text-muted">{subtitle}</p>}
          <div className="mt-6">{children}</div>
        </div>
      </div>
    </div>
  );
}

export function Notice({ kind, children }: { kind: "danger" | "ok" | "info"; children: ReactNode }) {
  return (
    <p role={kind === "danger" ? "alert" : "status"} className={`note note-${kind}`}>
      {children}
    </p>
  );
}

export function Field({
  id,
  label,
  error,
  hint,
  children,
}: {
  id: string;
  label: string;
  error?: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div>
      <label htmlFor={id} className="label">
        {label}
      </label>
      {children}
      {error ? (
        <p id={`${id}-error`} className="field-error">
          {error}
        </p>
      ) : (
        hint && <p className="hint">{hint}</p>
      )}
    </div>
  );
}

/** Props for an input inside `Field`, wiring its error for screen readers. */
export function invalid(id: string, error?: string) {
  return {
    id,
    "aria-invalid": error ? ("true" as const) : ("false" as const),
    "aria-describedby": error ? `${id}-error` : undefined,
  };
}

const ROLE_BADGE: Record<OrgRole, string> = {
  owner: "bg-accent-soft text-accent-strong",
  admin: "bg-warn-soft text-warn",
  manager: "bg-ok-soft text-ok",
  member: "bg-sunken text-muted",
};

export function RoleBadge({ role }: { role: OrgRole }) {
  return <span className={`badge ${ROLE_BADGE[role]}`}>{ROLE_LABELS[role]}</span>;
}

export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  return (
    <div className="mb-6 flex flex-wrap items-end justify-between gap-3">
      <div className="min-w-0">
        <h1 className="truncate text-xl font-semibold tracking-tight">{title}</h1>
        {subtitle && <p className="mt-1 text-sm text-muted">{subtitle}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
    </div>
  );
}

export function Section({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="panel">
      <header className="border-b border-line px-5 py-4">
        <h2 className="text-sm font-semibold">{title}</h2>
        {description && <p className="mt-0.5 text-sm text-muted">{description}</p>}
      </header>
      <div className="p-5">{children}</div>
    </section>
  );
}

export function EmptyState({ children }: { children: ReactNode }) {
  return <p className="py-6 text-center text-sm text-faint">{children}</p>;
}

export function LoadFailed({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="panel p-6 text-center">
      <p className="text-sm font-medium">Couldn&apos;t load this.</p>
      <p className="mt-1 text-sm text-muted">{message}</p>
      <button type="button" className="btn btn-secondary mt-4" onClick={onRetry}>
        Try again
      </button>
    </div>
  );
}
