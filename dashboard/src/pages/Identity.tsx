// The seller identity check, dashboard side: a person's history in the Users drawer and
// the review queue (a Moderation tab). Support sees statuses only; the verified name,
// year of birth, provider reference and the owner's notes arrive for the Owner alone.
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../api";
import { ago, day, words } from "../format";
import { useAction } from "../hooks";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/Modal";
import { DataTable } from "../ui/Table";
import { Card, CopyChip, EmptyState, ErrorNote, StatusBadge } from "../ui/kit";
import { SkeletonTable } from "../ui/Skeleton";

export interface IdentityCheck {
  id: string;
  user_id: string;
  status: "started" | "passed" | "failed" | "review" | "revoked";
  reason: string | null;
  started_at: string;
  decided_at: string | null;
  decided_by: string | null;
  provider?: string;
  provider_ref?: string;
  verified_name?: string;
  birth_year?: number;
  decision_note?: string;
}

const NOTE = "Reason (kept on the record, not in the audit log)";

type Target = { kind: "pass" | "fail" | "revoke"; check: IdentityCheck };

function Actions({ owner, check, onPick }: { owner: boolean; check: IdentityCheck; onPick: (t: Target) => void }) {
  if (!owner) return null;
  if (check.status === "review") {
    return (
      <span className="row" style={{ justifyContent: "flex-end" }}>
        <Button icon="check" onClick={() => onPick({ kind: "pass", check })}>Pass</Button>
        <Button variant="danger" icon="ban" onClick={() => onPick({ kind: "fail", check })}>Fail</Button>
      </span>
    );
  }
  if (check.status === "passed") {
    return (
      <span className="row" style={{ justifyContent: "flex-end" }}>
        <Button variant="danger" icon="undo" onClick={() => onPick({ kind: "revoke", check })}>Revoke</Button>
      </span>
    );
  }
  return null;
}

function Dialogs({ target, setTarget, invalidate }: { target: Target | null; setTarget: (t: Target | null) => void; invalidate: unknown[][] }) {
  const decide = useAction({
    run: ({ id, decision, reason }: { id: string; decision: "pass" | "fail"; reason: string }) => api.post(`/admin/identity/${id}/decide`, { decision, reason }),
    success: "Decision saved.",
    invalidate,
  });
  const revoke = useAction({ run: ({ id, reason }: { id: string; reason: string }) => api.post(`/admin/identity/${id}/revoke`, { reason }), success: "Check revoked. Their stores wait.", invalidate });
  return (
    <>
      <ConfirmDialog
        open={target?.kind === "pass"}
        title="Pass this check?"
        summary={<>Their stores can take orders (if the payout account is verified too). Say why you are satisfied.</>}
        confirmLabel="Pass"
        noteLabel={NOTE}
        onClose={() => setTarget(null)}
        onConfirm={async (reason) => {
          if (target) await decide.run({ id: target.check.id, decision: "pass", reason });
        }}
      />
      <ConfirmDialog
        open={target?.kind === "fail"}
        title="Fail this check?"
        summary={<>They are told the check was not completed and can ask support. Their NIN is not blocked.</>}
        confirmLabel="Fail"
        danger
        noteLabel={NOTE}
        onClose={() => setTarget(null)}
        onConfirm={async (reason) => {
          if (target) await decide.run({ id: target.check.id, decision: "fail", reason });
        }}
      />
      <ConfirmDialog
        open={target?.kind === "revoke"}
        title="Revoke this check?"
        summary={<>They become unverified, their stores stop taking orders, and the NIN stays taken so no other account can use it. For a scammer, ban the account instead: that also suspends their stores.</>}
        confirmLabel="Revoke"
        danger
        noteLabel={NOTE}
        onClose={() => setTarget(null)}
        onConfirm={async (reason) => {
          if (target) await revoke.run({ id: target.check.id, reason });
        }}
      />
    </>
  );
}

const label = (c: IdentityCheck) => words(c.reason ?? c.status);

/** The identity history of one person, for the Users drawer. */
export function IdentityPanel({ userId, owner }: { userId: string; owner: boolean }) {
  const q = useQuery({ queryKey: ["identity", userId], queryFn: () => api.get<{ items: IdentityCheck[] }>(`/admin/users/${userId}/identity`) });
  const [target, setTarget] = useState<Target | null>(null);
  const rows = q.data?.items ?? [];
  const current = rows.find((r) => r.status === "passed") ?? rows[0];
  return (
    <section aria-label="Identity check" style={{ marginTop: 18 }}>
      <h3>Identity check</h3>
      {q.isPending && <SkeletonTable rows={2} cols={3} label="Loading the identity check" />}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {q.data && rows.length === 0 && <p className="muted">Has not started the check.</p>}
      {current && (
        <dl className="kv">
          <dt>Status</dt>
          <dd><StatusBadge status={current.status} /></dd>
          {current.verified_name && (
            <>
              <dt>Verified name</dt>
              <dd>{current.verified_name}</dd>
            </>
          )}
          {current.birth_year && (
            <>
              <dt>Born</dt>
              <dd>{current.birth_year}</dd>
            </>
          )}
        </dl>
      )}
      {rows.length > 0 && (
        <DataTable
          label="Identity attempts"
          rows={rows}
          rowKey={(c) => c.id}
          columns={[
            { key: "s", header: "Result", cell: (c) => <StatusBadge status={c.status} /> },
            { key: "r", header: "Why", cell: (c) => (c.reason ? <span className="muted">{label(c)}</span> : "") },
            { key: "d", header: "Started", cell: (c) => day(c.started_at) },
            { key: "a", header: "", className: "actions", cell: (c) => <Actions owner={owner} check={c} onPick={setTarget} /> },
          ]}
        />
      )}
      {owner && rows.some((r) => r.decision_note) && (
        <ul className="hint" style={{ paddingLeft: 18 }}>
          {rows.filter((r) => r.decision_note).map((r) => (
            <li key={r.id}>{r.decision_note}</li>
          ))}
        </ul>
      )}
      <Dialogs target={target} setTarget={setTarget} invalidate={[["identity", userId], ["identity-review"]]} />
    </section>
  );
}

/** The queue of checks waiting for a person, a tab on the Moderation page. */
export function IdentityReview({ owner }: { owner: boolean }) {
  const q = useQuery({ queryKey: ["identity-review"], queryFn: () => api.get<{ items: IdentityCheck[] }>("/admin/identity/review-queue") });
  const [target, setTarget] = useState<Target | null>(null);
  return (
    <Card>
      {q.isPending && <SkeletonTable rows={3} cols={4} label="Loading identity checks" />}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {q.data && q.data.items.length === 0 && <EmptyState icon="check" title="Nothing to review" hint="Checks the provider could not decide, minors, name mismatches and sellers who ran out of attempts wait here." />}
      {q.data && q.data.items.length > 0 && (
        <DataTable
          label="Identity review"
          rows={q.data.items}
          rowKey={(c) => c.id}
          columns={[
            { key: "u", header: "Person", cell: (c) => <CopyChip value={c.user_id} label={c.user_id.slice(0, 8)} /> },
            { key: "r", header: "Why it is here", cell: (c) => label(c) },
            { key: "n", header: "Verified name", cell: (c) => c.verified_name ?? <span className="muted">{owner ? "none" : "hidden"}</span> },
            { key: "w", header: "Waiting", cell: (c) => ago(c.started_at) },
            { key: "x", header: "", className: "actions", cell: (c) => <Actions owner={owner} check={c} onPick={setTarget} /> },
          ]}
        />
      )}
      <Dialogs target={target} setTarget={setTarget} invalidate={[["identity-review"], ["identity"]]} />
    </Card>
  );
}
