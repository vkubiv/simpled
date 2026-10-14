import type { OrgRole } from "@simpled/backend-client";
import { useObserver, useSubtree } from "subtree.js/react";
import {
  EmptyState,
  Field,
  invalid,
  LoadFailed,
  Notice,
  PageHeader,
  RoleBadge,
  Section,
  Spinner,
} from "../../components/ui";
import { ROLE_DESCRIPTIONS, ROLE_LABELS } from "../../core/primitives/roles";
import { MembersActions, MembersState } from "./members-model";

export function MembersPage() {
  const state = useSubtree(MembersState);
  const actions = useSubtree(MembersActions);
  const observer = useObserver();
  const org = observer.watch(state.org);
  const load = observer.watch(state.load);
  const members = observer.watch(state.members);
  const invitations = observer.watch(state.invitations);
  const draft = observer.watch(state.invite);
  const inviteEmailError = observer.watch(state.inviteEmailError) ?? undefined;
  const inviteStatus = observer.watch(state.inviteStatus);
  const busyRow = observer.watch(state.busyRow);
  const rowError = observer.watch(state.rowError);
  if (!org) return null;

  const assignable = org.permissions.assignableRoles;
  const canManage = org.permissions.canManageMembers;

  return (
    <>
      <PageHeader title="Members" subtitle={`Everyone in ${org.name}, and what they can do.`} />

      {load.kind === "error" && <LoadFailed message={load.message} onRetry={() => void actions.reload()} />}
      {load.kind === "loading" && (
        <div className="flex justify-center p-10">
          <Spinner />
        </div>
      )}
      {load.kind === "loaded" && (
        <div className="space-y-6">
          {canManage && (
            <Section title="Invite someone" description="They get an email with a link that is valid for 7 days.">
              <form
                noValidate
                className="space-y-3"
                onSubmit={(e) => {
                  e.preventDefault();
                  void actions.sendInvite();
                }}
              >
                {inviteStatus.done && <Notice kind="ok">{inviteStatus.done}</Notice>}
                {inviteStatus.error && <Notice kind="danger">{inviteStatus.error}</Notice>}
                <div className="grid gap-3 sm:grid-cols-[1fr_11rem_auto] sm:items-start">
                  <Field id="invite-email" label="Email" error={inviteEmailError}>
                    <input
                      {...invalid("invite-email", inviteEmailError)}
                      type="email"
                      className="field"
                      placeholder="teammate@company.com"
                      value={draft.email}
                      onChange={(e) => actions.editInvite({ email: e.target.value })}
                    />
                  </Field>
                  <Field id="invite-role" label="Role">
                    <select
                      id="invite-role"
                      className="field"
                      value={draft.role}
                      onChange={(e) => actions.editInvite({ role: e.target.value as OrgRole })}
                    >
                      {assignable.map((r) => (
                        <option key={r} value={r}>
                          {ROLE_LABELS[r]}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <button type="submit" className="btn btn-primary sm:mt-[1.625rem]" disabled={inviteStatus.busy}>
                    {inviteStatus.busy ? "Sending…" : "Send invite"}
                  </button>
                </div>
                <p className="hint">{ROLE_DESCRIPTIONS[draft.role]}</p>
              </form>
            </Section>
          )}

          <Section title={`${members.length} ${members.length === 1 ? "member" : "members"}`}>
            {rowError && (
              <div className="mb-3">
                <Notice kind="danger">{rowError}</Notice>
              </div>
            )}
            <ul className="divide-y divide-line">
              {members.map((m) => (
                <li key={m.accountId} className="flex flex-wrap items-center gap-3 py-3">
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">
                      {m.name} {m.isMe && <span className="font-normal text-faint">(you)</span>}
                    </p>
                    <p className="truncate text-sm text-muted">{m.email}</p>
                  </div>
                  {m.canChangeRole && assignable.length > 0 ? (
                    <select
                      aria-label={`Role of ${m.name}`}
                      className="field w-36"
                      value={m.role}
                      disabled={busyRow !== null}
                      onChange={(e) => void actions.changeRole(m.accountId, e.target.value as OrgRole)}
                    >
                      {/* The current role stays listed even when the caller may not assign it. */}
                      {[...new Set([m.role, ...assignable])].map((r) => (
                        <option key={r} value={r} disabled={!assignable.includes(r)}>
                          {ROLE_LABELS[r]}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <RoleBadge role={m.role} />
                  )}
                  {m.canRemove && (
                    <button
                      type="button"
                      className="btn btn-danger btn-sm"
                      disabled={busyRow !== null}
                      onClick={() => {
                        const question = m.isMe ? `Leave ${org.name}?` : `Remove ${m.name} from ${org.name}?`;
                        if (window.confirm(question)) void actions.remove(m.accountId);
                      }}
                    >
                      {busyRow === m.accountId ? "…" : m.isMe ? "Leave" : "Remove"}
                    </button>
                  )}
                </li>
              ))}
            </ul>
          </Section>

          {canManage && (
            <Section title="Open invitations">
              {invitations.length === 0 ? (
                <EmptyState>No open invitations.</EmptyState>
              ) : (
                <ul className="divide-y divide-line">
                  {invitations.map((i) => (
                    <li key={i.id} className="flex flex-wrap items-center gap-3 py-3">
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-medium">{i.email}</p>
                        <p className="text-sm text-muted">
                          Invited by {i.invitedBy} ·{" "}
                          {i.expired ? (
                            <span className="text-warn">expired, invite again</span>
                          ) : (
                            `expires ${i.expiresAt.toLocaleDateString()}`
                          )}
                        </p>
                      </div>
                      <RoleBadge role={i.role} />
                      {assignable.includes(i.role) && (
                        <button
                          type="button"
                          className="btn btn-secondary btn-sm"
                          disabled={busyRow !== null}
                          onClick={() => void actions.revoke(i.id)}
                        >
                          {busyRow === `invitation:${i.id}` ? "…" : "Revoke"}
                        </button>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          )}
        </div>
      )}
    </>
  );
}
