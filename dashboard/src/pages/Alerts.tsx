import { useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { api, formatKobo } from "../api";
import { ALERT_LABELS, ago, dateTime, words } from "../format";
import { useAction } from "../hooks";
import { spring } from "../motion";
import { Button } from "../ui/Button";
import { Icon, type IconName } from "../ui/Icon";
import { ConfirmDialog } from "../ui/Modal";
import { Badge, Card, EmptyState, ErrorNote, Segmented } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonCards } from "../ui/Skeleton";

export interface Alert {
  alert_key: string;
  kind: string;
  subject_id: string | null;
  at: string;
  detail: Record<string, unknown>;
  acked_by: string | null;
  acked_at: string | null;
  note: string | null;
}

const ICONS: Record<string, IconName> = {
  "payout.provider_outcome_after_close": "money",
  "payout.amount_mismatch": "money",
  "payout.reversed_after_success": "undo",
  "courier.wallet_alert": "truck",
  "statement.mismatch": "audit",
  "order.no_delivery_method": "orders",
  "order.quote_reused": "orders",
  "order.repeated_handover_lockout": "ban",
};

/** A one-line, plain description of the numbers an alert carries. */
export function describe(a: Alert): string {
  const d = a.detail;
  const kobo = (v: unknown) => (typeof v === "number" ? formatKobo(v) : "?");
  switch (a.kind) {
    case "payout.amount_mismatch":
      return `We sent ${kobo(d.expected_kobo)}, the provider says ${kobo(d.reported_kobo)}.`;
    case "payout.reversed_after_success":
      return `${kobo(d.amount_kobo)} came back.`;
    case "payout.provider_outcome_after_close":
      return `Outcome: ${words(String(d.outcome ?? "unknown"))} on a ${words(String(d.reference_kind ?? "closed"))} payout.`;
    case "courier.wallet_alert":
      return words(String(d.event ?? "wallet warning"));
    case "statement.mismatch": {
      const diff = (d.diff ?? {}) as Record<string, number>;
      const parts = Object.entries(diff).filter(([, v]) => v !== 0).map(([k, v]) => `${words(k.replace("_kobo", ""))} ${formatKobo(v)}`);
      return `${String(d.day ?? "")}: ${parts.join(", ") || "differs"}`;
    }
    case "order.repeated_handover_lockout":
      return `${String(d.lockouts ?? "Several")} lockouts so far.`;
    default:
      return "";
  }
}

export function Alerts() {
  const [state, setState] = useState<"open" | "acked">("open");
  const q = useQuery({ queryKey: ["alerts", state], queryFn: () => api.get<{ items: Alert[] }>("/admin/alerts", { state, limit: 100 }) });
  const [target, setTarget] = useState<Alert | null>(null);
  const ack = useAction({ run: ({ key, note }: { key: string; note: string }) => api.post("/admin/alerts/ack", { alert_key: key, note }), success: "Acknowledged.", invalidate: [["alerts"], ["reconciliation"]] });
  const items = q.data?.items ?? [];

  return (
    <>
      <PageHeader
        title="Alerts"
        subtitle="Things the system noticed and cannot settle. They stay until you acknowledge them."
        actions={
          <Segmented id="alerts" label="Show" value={state} onChange={setState} options={[{ value: "open", label: "Open" }, { value: "acked", label: "Acknowledged" }]} />
        }
      />
      <div style={{ marginTop: 16, display: "grid", gap: 12 }}>
        {q.isPending && <SkeletonCards count={3} label="Loading alerts" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {q.data && items.length === 0 && (
          <Card>
            <EmptyState icon="check" title={state === "open" ? "All clear" : "Nothing acknowledged yet"} hint={state === "open" ? "No open alerts." : undefined} />
          </Card>
        )}
        <AnimatePresence initial={false} mode="popLayout">
          {items.map((a) => {
            const label = ALERT_LABELS[a.kind] ?? { title: words(a.kind), hint: "" };
            return (
              <motion.div
                key={a.alert_key}
                layout
                initial={{ opacity: 0, y: 12 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, x: 120, transition: { duration: 0.22 } }}
                transition={spring.snappy}
              >
                <Card className="alert-card">
                  <motion.span className="alert-icon" initial={{ rotate: -20, scale: 0.6 }} animate={{ rotate: 0, scale: 1 }} transition={spring.bouncy}>
                    <Icon name={ICONS[a.kind] ?? "alert"} size={22} active />
                  </motion.span>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div className="row">
                      <strong>{label.title}</strong>
                      <span className="muted" title={dateTime(a.at)}>{ago(a.at)}</span>
                    </div>
                    <div>{describe(a)}</div>
                    <div className="hint">{label.hint}</div>
                    {a.acked_at && (
                      <div className="row" style={{ marginTop: 6 }}>
                        <Badge tone="good">acknowledged {ago(a.acked_at)}</Badge>
                        <span className="muted">“{a.note}”</span>
                      </div>
                    )}
                  </div>
                  {!a.acked_at && (
                    <Button icon="check" onClick={() => setTarget(a)}>
                      Acknowledge
                    </Button>
                  )}
                </Card>
              </motion.div>
            );
          })}
        </AnimatePresence>
      </div>
      <ConfirmDialog
        open={target !== null}
        title="Acknowledge this alert?"
        summary={target && <><strong>{(ALERT_LABELS[target.kind] ?? { title: words(target.kind) }).title}</strong>. Say what you did or found; it is kept with the acknowledgement.</>}
        confirmLabel="Acknowledge"
        noteLabel="What did you do?"
        onClose={() => setTarget(null)}
        onConfirm={async (note) => {
          if (target) await ack.run({ key: target.alert_key, note });
        }}
      />
    </>
  );
}
