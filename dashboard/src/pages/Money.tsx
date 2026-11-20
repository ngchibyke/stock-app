import { useQuery } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { api, formatKobo } from "../api";
import { dateTime, words } from "../format";
import { useAction } from "../hooks";
import { Button, IconButton } from "../ui/Button";
import { ConfirmDialog } from "../ui/Modal";
import { DataTable } from "../ui/Table";
import { Badge, Card, CopyChip, EmptyState, ErrorNote, Money, StatusBadge, Tabs } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonTable } from "../ui/Skeleton";

interface Payout {
  id: string;
  order_id: string;
  store_id: string;
  reference: string;
  status: string;
  amount_kobo: number;
  fee_kobo: number;
  attempts: number;
  last_error: string | null;
  account_last4: string | null;
  reversed: boolean;
  settled_by: string | null;
  updated_at: string;
}
interface Refund {
  id: string;
  order_id: string;
  reference: string;
  status: string;
  amount_kobo: number;
  attempts: number;
  last_error: string | null;
  updated_at: string;
}
interface Checkout {
  id: string;
  reference: string;
  status: string;
  provider: string;
  total_kobo: number;
  provider_flagged: boolean;
  resolved: boolean;
  paid_at: string | null;
  created_at: string;
}
interface Shipment {
  alert_key: string;
  id: string | null;
  order_id: string | null;
  status: string;
  order_status: string | null;
  provider_order_id: string | null;
  quoted_kobo: number | null;
  charged_kobo: number | null;
  attempts: number;
  last_error: string | null;
  attention: string | null;
  price_above_quote: boolean;
  updated_at: string;
}
interface OrderAttention {
  alert_key: string;
  order_id: string;
  attention: string;
  paid_at: string;
}
export interface Reconciliation {
  payouts: Payout[];
  refunds: Refund[];
  flagged_checkouts: Checkout[];
  mismatched_checkouts: Checkout[];
  blocked_payouts: Payout[];
  shipments_attention: Shipment[];
  orders_attention: OrderAttention[];
}
interface StatementCheck {
  id: string;
  day: string;
  status: string;
  diff: Record<string, number> | null;
  checked_at: string;
}

type Tab = "payouts" | "refunds" | "payments" | "shipments" | "orders" | "statements";
const TABS: Tab[] = ["payouts", "refunds", "payments", "shipments", "orders", "statements"];

/** A confirm dialog with the optional extras money overrides need. */
function MoneyDialog(props: {
  open: boolean;
  title: string;
  summary: ReactNode;
  label: string;
  danger?: boolean;
  needReference?: boolean;
  attest?: string;
  noNote?: boolean;
  onClose: () => void;
  onSubmit: (v: { note: string; reference: string; attest: boolean }) => Promise<void>;
}) {
  const [reference, setReference] = useState("");
  const [attest, setAttest] = useState(false);
  return (
    <ConfirmDialog
      open={props.open}
      title={props.title}
      summary={props.summary}
      confirmLabel={props.label}
      danger={props.danger}
      noteLabel={props.noNote ? false : undefined}
      blockedReason={props.needReference && !reference.trim() ? "Enter the provider reference you checked to continue." : undefined}
      onClose={props.onClose}
      onConfirm={async (note) => {
        await props.onSubmit({ note, reference: reference.trim(), attest });
      }}
    >
      {props.needReference && (
        <label className="field">
          <span>Provider reference you checked (not an account number)</span>
          <input value={reference} onChange={(e) => setReference(e.target.value)} placeholder="e.g. TRX_8H2KD9" />
        </label>
      )}
      {props.attest && (
        <label className="row" style={{ marginTop: 14 }}>
          <input type="checkbox" checked={attest} onChange={(e) => setAttest(e.target.checked)} style={{ width: "auto" }} />
          <span>{props.attest}</span>
        </label>
      )}
    </ConfirmDialog>
  );
}

export function MoneyPage() {
  const rec = useQuery({ queryKey: ["reconciliation"], queryFn: () => api.get<Reconciliation>("/admin/reconciliation") });
  // The tab lives in the address (?tab=refunds), so the overview can link straight to it.
  const [params, setParams] = useSearchParams();
  const asked = params.get("tab");
  const tab: Tab = TABS.includes(asked as Tab) ? (asked as Tab) : "payouts";
  const setTab = (t: Tab) => setParams(t === "payouts" ? {} : { tab: t }, { replace: true });
  const d = rec.data;
  const count = (n: number | undefined) => n ?? 0;

  return (
    <>
      <PageHeader
        title="Money and reconciliation"
        subtitle="Everything the system cannot finish by itself, oldest first."
        actions={
          <Button icon="refresh" onClick={() => void rec.refetch()} loading={rec.isFetching && !rec.isPending}>
            Refresh
          </Button>
        }
      />
      <div>
        <Tabs
          id="money"
          value={tab}
          onChange={setTab}
          tabs={[
            { value: "payouts", label: "Payouts", count: count(d?.payouts.length) + count(d?.blocked_payouts.length) },
            { value: "refunds", label: "Refunds", count: count(d?.refunds.length) },
            { value: "payments", label: "Payments", count: count(d?.flagged_checkouts.length) + count(d?.mismatched_checkouts.length) },
            { value: "shipments", label: "Shipments", count: count(d?.shipments_attention.length) },
            { value: "orders", label: "Orders", count: count(d?.orders_attention.length) },
            { value: "statements", label: "Statements" },
          ]}
        />
      </div>
      <div className="stack" style={{ marginTop: 16 }}>
        {rec.isPending && <Card><SkeletonTable rows={5} cols={6} label="Loading reconciliation" /></Card>}
        {rec.isError && <ErrorNote error={rec.error} onRetry={() => void rec.refetch()} />}
        {d && tab === "payouts" && <Payouts rec={d} />}
        {d && tab === "refunds" && <Refunds rows={d.refunds} />}
        {d && tab === "payments" && <Payments rec={d} />}
        {d && tab === "shipments" && <Shipments rows={d.shipments_attention} />}
        {d && tab === "orders" && <OrdersAttention rows={d.orders_attention} />}
        {tab === "statements" && <Statements />}
      </div>
    </>
  );
}

const Clear = ({ what }: { what: string }) => <EmptyState icon="check" title={`No ${what}`} hint="Nothing is waiting here." />;

type PayoutAct = { kind: "paid" | "requeue"; row: Payout } | null;

function Payouts({ rec }: { rec: Reconciliation }) {
  const [act, setAct] = useState<PayoutAct>(null);
  const inv = [["reconciliation"]];
  const paid = useAction({ run: ({ id, ...b }: { id: string; note: string; provider_reference: string; attest_provider_checked?: boolean }) => api.post(`/admin/payouts/${id}/mark-paid`, b), success: "Marked as paid by hand.", invalidate: inv });
  const requeue = useAction({ run: ({ id, ...b }: { id: string; note: string; attest_provider_checked?: boolean }) => api.post(`/admin/payouts/${id}/requeue`, b), success: "Queued to be sent again under a new reference.", invalidate: inv });
  const repoll = useAction<string, { status: string }>({ run: (id) => api.post(`/admin/payouts/${id}/repoll`), success: (r) => `Asked the provider. Payout is ${words(r.status)}.`, invalidate: inv });
  const r = act?.row;

  return (
    <>
      <Card>
        <h2>Needs you</h2>
        {rec.payouts.length === 0 ? (
          <Clear what="payouts waiting" />
        ) : (
          <DataTable
            label="Payouts needing attention"
            rows={rec.payouts}
            rowKey={(p) => p.id}
            columns={[
              { key: "o", header: "Order", cell: (p) => <span className="mono">{p.order_id.slice(0, 8)}</span> },
              { key: "s", header: "Status", cell: (p) => <span className="row"><StatusBadge status={p.status} />{p.reversed && <Badge tone="bad">reversed</Badge>}</span> },
              { key: "a", header: "Amount", className: "num", cell: (p) => <Money kobo={p.amount_kobo} /> },
              { key: "b", header: "Account", cell: (p) => (p.account_last4 ? <span className="mono">••••{p.account_last4}</span> : "–") },
              { key: "e", header: "Last error", cell: (p) => <span className="muted">{p.last_error ? words(p.last_error) : "–"}</span> },
              {
                key: "x",
                header: "",
                className: "actions",
                cell: (p) => (
                  <span className="row" style={{ justifyContent: "flex-end" }}>
                    <IconButton icon="refresh" label="Ask provider" disabled={repoll.busy} onClick={() => void repoll.run(p.id).catch(() => {})} />
                    <Button icon="undo" onClick={() => setAct({ kind: "requeue", row: p })}>Send again</Button>
                    <Button variant="primary" icon="check" onClick={() => setAct({ kind: "paid", row: p })}>Paid by hand</Button>
                  </span>
                ),
              },
            ]}
          />
        )}
      </Card>
      {rec.blocked_payouts.length > 0 && (
        <Card>
          <h2>Waiting 7+ days on something only a person can fix</h2>
          <p className="muted">A held order, a flagged payment, or a payout account that is missing or not verified.</p>
          <DataTable
            label="Blocked payouts"
            rows={rec.blocked_payouts}
            rowKey={(p) => p.id}
            columns={[
              { key: "o", header: "Order", cell: (p) => <span className="mono">{p.order_id.slice(0, 8)}</span> },
              { key: "a", header: "Amount", className: "num", cell: (p) => <Money kobo={p.amount_kobo} /> },
              { key: "u", header: "Waiting since", cell: (p) => dateTime(p.updated_at) },
            ]}
          />
        </Card>
      )}
      <MoneyDialog
        open={r !== undefined && act?.kind === "paid"}
        title="Mark this payout as paid?"
        summary={r && <>You paid <strong><Money kobo={r.amount_kobo} /></strong> to the seller outside the system. The ledger will show it paid. If the provider also pays it, that is a double payment, and you will be alerted.</>}
        label="Mark paid"
        needReference
        attest="I checked the provider's dashboard and this payout was not paid there."
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r) await paid.run({ id: r.id, note: v.note, provider_reference: v.reference, attest_provider_checked: v.attest || undefined });
        }}
      />
      <MoneyDialog
        open={r !== undefined && act?.kind === "requeue"}
        title="Send this payout again?"
        summary={r && <>The system will send <strong><Money kobo={r.amount_kobo} /></strong> again under a new reference. Only do this if the first attempt was reversed or failed.</>}
        label="Send again"
        attest="I checked the provider's dashboard and the old payout was not paid."
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r) await requeue.run({ id: r.id, note: v.note, attest_provider_checked: v.attest || undefined });
        }}
      />
    </>
  );
}

function Refunds({ rows }: { rows: Refund[] }) {
  const [row, setRow] = useState<Refund | null>(null);
  const mark = useAction({ run: ({ id, ...b }: { id: string; note: string; provider_reference: string }) => api.post(`/admin/refunds/${id}/mark-refunded`, b), success: "Marked as refunded by hand.", invalidate: [["reconciliation"]] });
  return (
    <Card>
      {rows.length === 0 ? (
        <Clear what="stuck refunds" />
      ) : (
        <DataTable
          label="Refunds"
          rows={rows}
          rowKey={(x) => x.id}
          columns={[
            { key: "o", header: "Order", cell: (x) => <span className="mono">{x.order_id.slice(0, 8)}</span> },
            { key: "s", header: "Status", cell: (x) => <StatusBadge status={x.status} /> },
            { key: "a", header: "Amount", className: "num", cell: (x) => <Money kobo={x.amount_kobo} /> },
            { key: "n", header: "Tries", cell: (x) => x.attempts },
            { key: "e", header: "Last error", cell: (x) => <span className="muted">{x.last_error ? words(x.last_error) : "–"}</span> },
            { key: "x", header: "", className: "actions", cell: (x) => <Button variant="primary" icon="check" onClick={() => setRow(x)}>Refunded by hand</Button> },
          ]}
        />
      )}
      <MoneyDialog
        open={row !== null}
        title="Mark this refund as paid?"
        summary={row && <>You returned <strong><Money kobo={row.amount_kobo} /></strong> to the buyer outside the system.</>}
        label="Mark refunded"
        needReference
        onClose={() => setRow(null)}
        onSubmit={async (v) => {
          if (row) await mark.run({ id: row.id, note: v.note, provider_reference: v.reference });
        }}
      />
    </Card>
  );
}

function Payments({ rec }: { rec: Reconciliation }) {
  const [act, setAct] = useState<{ kind: "clear" | "resolve"; row: Checkout } | null>(null);
  const inv = [["reconciliation"]];
  const clear = useAction({ run: ({ id, note }: { id: string; note: string }) => api.post(`/admin/checkouts/${id}/clear-flag`, { note }), success: "Flag cleared.", invalidate: inv });
  const resolve = useAction({ run: ({ id, ...b }: { id: string; note: string; provider_reference: string }) => api.post(`/admin/checkouts/${id}/resolve`, b), success: "Recorded as handled.", invalidate: inv });
  const table = (rows: Checkout[], label: string) => (
    <DataTable
      label={label}
      rows={rows}
      rowKey={(c) => c.id}
      columns={[
        { key: "r", header: "Reference", cell: (c) => <CopyChip value={c.reference} /> },
        { key: "s", header: "Status", cell: (c) => <span className="row"><StatusBadge status={c.status} />{c.provider_flagged && <Badge tone="bad">flagged</Badge>}</span> },
        { key: "t", header: "Total", className: "num", cell: (c) => <Money kobo={c.total_kobo} /> },
        { key: "p", header: "Paid", cell: (c) => dateTime(c.paid_at) },
        {
          key: "x",
          header: "",
          className: "actions",
          cell: (c) => (
            <span className="row" style={{ justifyContent: "flex-end" }}>
              {c.provider_flagged && <Button icon="flag" onClick={() => setAct({ kind: "clear", row: c })}>Clear flag</Button>}
              <Button variant="primary" icon="check" onClick={() => setAct({ kind: "resolve", row: c })}>Handled</Button>
            </span>
          ),
        },
      ]}
    />
  );
  const r = act?.row;
  return (
    <>
      <Card>
        <h2>Flagged by the provider</h2>
        {rec.flagged_checkouts.length === 0 ? <Clear what="flagged payments" /> : table(rec.flagged_checkouts, "Flagged payments")}
      </Card>
      <Card>
        <h2>Paid a different amount</h2>
        {rec.mismatched_checkouts.length === 0 ? <Clear what="amount mismatches" /> : table(rec.mismatched_checkouts, "Amount mismatches")}
      </Card>
      <ConfirmDialog
        open={act?.kind === "clear"}
        title="Clear the provider's flag?"
        summary={r && <>Payment <span className="mono">{r.reference}</span> for <Money kobo={r.total_kobo} /> is released to normal handling. Auto-release and payouts can resume.</>}
        confirmLabel="Clear flag"
        onClose={() => setAct(null)}
        onConfirm={async (note) => {
          if (r) await clear.run({ id: r.id, note });
        }}
      />
      <MoneyDialog
        open={act?.kind === "resolve"}
        title="Mark this payment as handled?"
        summary={r && <>You dealt with <span className="mono">{r.reference}</span> (<Money kobo={r.total_kobo} />) outside the system, for example by refunding it at the provider.</>}
        label="Mark handled"
        needReference
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r) await resolve.run({ id: r.id, note: v.note, provider_reference: v.reference });
        }}
      />
    </>
  );
}

function Shipments({ rows }: { rows: Shipment[] }) {
  const [act, setAct] = useState<{ kind: "rebook" | "cancel" | "ack"; row: Shipment } | null>(null);
  const inv = [["reconciliation"], ["alerts"]];
  const rebook = useAction({ run: ({ id, ...b }: { id: string; note: string; confirm_not_booked?: boolean }) => api.post(`/admin/shipments/${id}/rebook`, b), success: "Back in the queue. The next job run books it.", invalidate: inv });
  const cancel = useAction({ run: ({ id, note }: { id: string; note: string }) => api.post(`/admin/shipments/${id}/retry-cancel`, { note }), success: "Cancel will be tried again.", invalidate: inv });
  const ack = useAction({ run: ({ key, note }: { key: string; note: string }) => api.post("/admin/alerts/ack", { alert_key: key, note }), success: "Acknowledged.", invalidate: inv });
  const r = act?.row;
  return (
    <Card>
      {rows.length === 0 ? (
        <Clear what="shipments needing a person" />
      ) : (
        <DataTable
          label="Shipments needing attention"
          rows={rows}
          rowKey={(s) => s.alert_key}
          columns={[
            { key: "o", header: "Order", cell: (s) => <span className="mono">{s.order_id ? s.order_id.slice(0, 8) : "unknown label"}</span> },
            { key: "s", header: "Shipment", cell: (s) => <span className="row"><StatusBadge status={s.status} />{s.attention && <Badge tone="warn">{words(s.attention)}</Badge>}{s.price_above_quote && <Badge tone="bad">above quote</Badge>}</span> },
            { key: "q", header: "Quoted / charged", cell: (s) => <span className="muted">{formatKobo(s.quoted_kobo)} / {formatKobo(s.charged_kobo)}</span> },
            { key: "e", header: "Last error", cell: (s) => <span className="muted">{s.last_error ? words(s.last_error) : "–"}</span> },
            {
              key: "x",
              header: "",
              className: "actions",
              cell: (s) => (
                <span className="row" style={{ justifyContent: "flex-end" }}>
                  {s.id && ["booking_failed", "booking_unknown"].includes(s.status) && <Button variant="primary" icon="refresh" onClick={() => setAct({ kind: "rebook", row: s })}>Re-book</Button>}
                  {s.id && s.status === "cancel_failed" && <Button icon="refresh" onClick={() => setAct({ kind: "cancel", row: s })}>Retry cancel</Button>}
                  <Button variant="ghost" icon="check" onClick={() => setAct({ kind: "ack", row: s })}>Acknowledge</Button>
                </span>
              ),
            },
          ]}
        />
      )}
      <MoneyDialog
        open={act?.kind === "rebook"}
        title="Book this courier again?"
        summary={r && (r.status === "booking_unknown" ? <>The first attempt may have gone through and charged the wallet. Only continue if you checked the courier dashboard and there is no label for this order.</> : <>The courier refused the booking, often an empty wallet. Fund the wallet first, then book again. A stale quote needs a new quote instead, which is not built yet.</>)}
        label="Book again"
        attest={r?.status === "booking_unknown" ? "I checked the courier dashboard: no label exists for this order." : undefined}
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r?.id) await rebook.run({ id: r.id, note: v.note, confirm_not_booked: r.status === "booking_unknown" ? v.attest : undefined });
        }}
      />
      <MoneyDialog
        open={act?.kind === "cancel"}
        title="Try the cancel again?"
        summary={<>The courier refused to cancel this shipment. Try once more.</>}
        label="Retry cancel"
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r?.id) await cancel.run({ id: r.id, note: v.note });
        }}
      />
      <MoneyDialog
        open={act?.kind === "ack"}
        title="Acknowledge this?"
        summary={<>It leaves this list. If the state changes it comes back.</>}
        label="Acknowledge"
        onClose={() => setAct(null)}
        onSubmit={async (v) => {
          if (r) await ack.run({ key: r.alert_key, note: v.note });
        }}
      />
    </Card>
  );
}

function OrdersAttention({ rows }: { rows: OrderAttention[] }) {
  const [row, setRow] = useState<OrderAttention | null>(null);
  const ack = useAction({ run: ({ key, note }: { key: string; note: string }) => api.post("/admin/alerts/ack", { alert_key: key, note }), success: "Acknowledged.", invalidate: [["reconciliation"], ["alerts"]] });
  return (
    <Card>
      {rows.length === 0 ? (
        <Clear what="orders needing a decision" />
      ) : (
        <DataTable
          label="Orders needing a decision"
          rows={rows}
          rowKey={(o) => o.alert_key}
          columns={[
            { key: "o", header: "Order", cell: (o) => <CopyChip value={o.order_id} label={o.order_id.slice(0, 8)} /> },
            { key: "w", header: "Why", cell: (o) => <Badge tone="warn">{words(o.attention)}</Badge> },
            { key: "p", header: "Paid", cell: (o) => dateTime(o.paid_at) },
            { key: "x", header: "", className: "actions", cell: (o) => <Button variant="ghost" icon="check" onClick={() => setRow(o)}>Acknowledge</Button> },
          ]}
        />
      )}
      <ConfirmDialog
        open={row !== null}
        title="Acknowledge this order?"
        summary={<>Open the order under Orders to release or refund it. Acknowledging only clears it from this list.</>}
        confirmLabel="Acknowledge"
        onClose={() => setRow(null)}
        onConfirm={async (note) => {
          if (row) await ack.run({ key: row.alert_key, note });
        }}
      />
    </Card>
  );
}

function Statements() {
  const q = useQuery({ queryKey: ["statements"], queryFn: () => api.get<{ items: StatementCheck[] }>("/admin/statement-checks") });
  const [dayText, setDayText] = useState("");
  const run = useAction({ run: () => api.post<StatementCheck>("/admin/statement-checks", { day: dayText }), success: (r) => `Checked ${r.day}: ${r.status}.`, invalidate: [["statements"], ["alerts"]] });
  return (
    <Card>
      <div className="toolbar" style={{ marginTop: 0 }}>
        <label className="row muted">
          Check a finished day <input type="date" value={dayText} onChange={(e) => setDayText(e.target.value)} style={{ width: "auto" }} />
        </label>
        <Button variant="primary" icon="refresh" disabled={!dayText} loading={run.busy} done={run.done} onClick={() => void run.run(undefined).catch(() => {})}>
          Check now
        </Button>
      </div>
      {q.isPending && <SkeletonTable rows={3} cols={4} label="Loading statement checks" />}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {q.data && q.data.items.length === 0 && <EmptyState icon="audit" title="No statement checks yet" hint="Checks run daily once a provider statement source is installed." />}
      {q.data && q.data.items.length > 0 && (
        <DataTable
          label="Statement checks"
          rows={q.data.items}
          rowKey={(c) => c.id}
          columns={[
            { key: "d", header: "Day", cell: (c) => c.day },
            { key: "s", header: "Result", cell: (c) => <StatusBadge status={c.status} /> },
            { key: "f", header: "Difference", cell: (c) => (c.diff ? <span className="mono">{Object.entries(c.diff).filter(([, v]) => v !== 0).map(([k, v]) => `${words(k.replace("_kobo", ""))} ${formatKobo(v)}`).join(" · ") || "none"}</span> : "–") },
            { key: "t", header: "Checked", cell: (c) => dateTime(c.checked_at) },
          ]}
        />
      )}
    </Card>
  );
}
