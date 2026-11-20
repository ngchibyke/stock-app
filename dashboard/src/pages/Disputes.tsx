import { useQuery } from "@tanstack/react-query";
import { motion } from "motion/react";
import { useEffect, useState } from "react";
import { api, formatKobo } from "../api";
import { useAuth } from "../auth";
import { ago, dateTime, words } from "../format";
import { useAction } from "../hooks";
import { spring } from "../motion";
import { Button } from "../ui/Button";
import { Drawer } from "../ui/Drawer";
import { Icon } from "../ui/Icon";
import { Modal } from "../ui/Modal";
import { DataTable } from "../ui/Table";
import { Badge, Card, EmptyState, ErrorNote, Money, StatusBadge, Tabs } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonDetail, SkeletonTable } from "../ui/Skeleton";

interface QueueRow {
  id: string;
  order_id: string;
  status: string;
  reason: string;
  opened_at: string;
  seller_respond_by: string;
  items_total_kobo: number;
}
interface Statement {
  body: string;
  photo_keys: string[];
  at: string;
}
export interface DisputeScreen {
  dispute: {
    id: string;
    order_id: string;
    status: string;
    reason: string;
    opened_at: string;
    seller_respond_by: string;
    seller_missed_deadline: boolean;
    buyer_statement: Statement;
    seller_statement: Statement | null;
    return_request: { paid_by: string; requested_at: string | null; received_at: string | null } | null;
    decision: { decision: string; refund_kobo: number | null; decided_at: string | null } | null;
  };
  order: { order_id: string; status: string; items_total_kobo: number; delivery_fee_kobo: number; held: boolean; provider_flagged: boolean; payout_status: string | null };
  payment: { checkout_status: string; provider: string; provider_flagged: boolean; paid_at: string | null; total_kobo: number };
  ledger: { at: string; account: string; amount_kobo: number; kind: string }[];
  timeline: { key: string; label: string; at: string | null; current: boolean }[];
  delivery_events: { source: string; status: string; occurred_at: string | null; received_at: string }[];
  handover: { applies: boolean; issued_at: string | null; wrong_entries: number; locked_until: string | null; delivered_with_code: boolean };
  chat: { seq: number; from: "buyer" | "seller"; body: string; sent_at: string }[];
  flagged?: { from: "buyer" | "seller"; severity: string; category: string; original: string; at: string }[];
}

type Status = "awaiting_decision" | "awaiting_seller" | "awaiting_return" | "decided" | "withdrawn";
const TABS: { value: Status; label: string }[] = [
  { value: "awaiting_decision", label: "Ready to decide" },
  { value: "awaiting_seller", label: "Seller's turn" },
  { value: "awaiting_return", label: "Return pending" },
  { value: "decided", label: "Decided" },
  { value: "withdrawn", label: "Withdrawn" },
];
const SELLER_WINDOW_MS = 48 * 3600 * 1000;

/** Time left for the seller, ticking each second, with a bar that drains. */
export function Countdown({ until, now = Date.now() }: { until: string; now?: number }) {
  const [t, setT] = useState(now);
  useEffect(() => {
    const id = setInterval(() => setT(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  const left = Date.parse(until) - t;
  if (left <= 0) return <Badge tone="bad">deadline passed</Badge>;
  const h = Math.floor(left / 3_600_000);
  const m = Math.floor((left % 3_600_000) / 60_000);
  return (
    <span className="countdown" aria-label={`${h} hours ${m} minutes left`}>
      <span className="mono">{h}h {String(m).padStart(2, "0")}m</span>
      <span className="progress"><motion.span animate={{ scaleX: Math.min(1, left / SELLER_WINDOW_MS) }} transition={spring.gentle} /></span>
    </span>
  );
}

export function Disputes() {
  const [tab, setTab] = useState<Status>("awaiting_decision");
  const [open, setOpen] = useState<string | null>(null);
  const q = useQuery({ queryKey: ["disputes", tab], queryFn: () => api.get<{ items: QueueRow[] }>("/admin/disputes", { status: tab, limit: 100 }) });
  const rows = q.data?.items ?? [];
  return (
    <>
      <PageHeader title="Disputes" subtitle="Oldest first. Open one to see both sides, the order and the chat." />
      <div style={{ marginTop: 14 }}>
        <Tabs id="disputes" value={tab} onChange={setTab} tabs={TABS} />
      </div>
      <Card className="" >
        {q.isPending && <SkeletonTable rows={4} cols={5} label="Loading disputes" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {q.data && rows.length === 0 && <EmptyState icon="dispute" title="Nothing here" hint="No disputes in this state." />}
        {rows.length > 0 && (
          <DataTable
            label="Disputes"
            rows={rows}
            rowKey={(d) => d.id}
            onRowClick={(d) => setOpen(d.id)}
            columns={[
              { key: "o", header: "Order", cell: (d) => <span className="mono">{d.order_id.slice(0, 8)}</span> },
              { key: "r", header: "Reason", cell: (d) => <strong style={{ textTransform: "capitalize" }}>{words(d.reason)}</strong> },
              { key: "a", header: "Items", className: "num", cell: (d) => <Money kobo={d.items_total_kobo} /> },
              { key: "w", header: "Opened", cell: (d) => ago(d.opened_at) },
              { key: "c", header: tab === "awaiting_seller" ? "Seller has" : "Status", cell: (d) => (d.status === "awaiting_seller" ? <Countdown until={d.seller_respond_by} /> : <StatusBadge status={d.status} />) },
            ]}
          />
        )}
      </Card>
      <DisputeDrawer id={open} onClose={() => setOpen(null)} />
    </>
  );
}

function Statements({ d }: { d: DisputeScreen["dispute"] }) {
  const one = (who: "buyer" | "seller", s: Statement | null) => (
    <motion.div className={`statement ${who}`} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <strong style={{ textTransform: "capitalize" }}>{who}</strong>
        {s && <span className="muted">{dateTime(s.at)}</span>}
      </div>
      {s ? (
        <>
          <p>{s.body}</p>
          {s.photo_keys.length > 0 && <div className="hint">{s.photo_keys.length} photo(s) attached</div>}
        </>
      ) : (
        <p className="muted">{d.seller_missed_deadline ? "The seller did not respond in time." : "The seller has not responded yet."}</p>
      )}
    </motion.div>
  );
  return (
    <div className="two">
      {one("buyer", d.buyer_statement)}
      {one("seller", d.seller_statement)}
    </div>
  );
}

function DisputeDrawer({ id, onClose }: { id: string | null; onClose: () => void }) {
  const { user } = useAuth();
  const owner = user?.role === "owner";
  const q = useQuery({ queryKey: ["dispute", id], queryFn: () => api.get<DisputeScreen>(`/admin/disputes/${id}`), enabled: id !== null });
  const [mode, setMode] = useState<"decide" | "return" | null>(null);
  const s = q.data;
  const d = s?.dispute;
  const isOpen = !!d && ["awaiting_seller", "awaiting_return", "awaiting_decision"].includes(d.status);
  const sellerClock = !!d && d.status === "awaiting_seller" && Date.parse(d.seller_respond_by) > Date.now();

  return (
    <>
      <Drawer wide open={id !== null} title={d ? `Dispute · ${words(d.reason)}` : "Dispute"} onClose={onClose}>
        {q.isPending && <SkeletonDetail rows={8} label="Loading the dispute" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {s && d && (
          <>
            <div className="row">
              <StatusBadge status={d.status} />
              {d.status === "awaiting_seller" && <Countdown until={d.seller_respond_by} />}
              {s.order.held && <Badge tone="warn">order held</Badge>}
              {s.payment.provider_flagged && <Badge tone="bad">payment flagged</Badge>}
              <span className="muted">Opened {ago(d.opened_at)}</span>
            </div>

            <Statements d={d} />

            <Card>
              <h2>Order</h2>
              <dl className="kv" style={{ marginTop: 8 }}>
                <dt>Status</dt><dd><StatusBadge status={s.order.status} /></dd>
                <dt>Items</dt><dd><Money kobo={s.order.items_total_kobo} /></dd>
                <dt>Delivery fee</dt><dd><Money kobo={s.order.delivery_fee_kobo} /></dd>
                <dt>Paid</dt><dd>{dateTime(s.payment.paid_at)} via {s.payment.provider}</dd>
                {s.handover.applies && (
                  <>
                    <dt>Handover code</dt>
                    <dd>
                      {s.handover.delivered_with_code ? "Delivery confirmed with the code. " : "Not used. "}
                      {s.handover.wrong_entries > 0 && <Badge tone="warn">{s.handover.wrong_entries} wrong entries</Badge>}
                    </dd>
                  </>
                )}
              </dl>
              <div className="stepper" role="list" aria-label="Order progress" style={{ marginTop: 12 }}>
                {s.timeline.map((t, i) => (
                  <motion.div key={t.key} role="listitem" className={`step ${t.at ? "done" : ""} ${t.current ? "now" : ""}`} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: i * 0.05 }}>
                    <span className="pip">{t.at ? <Icon name="check" size={12} interactive={false} /> : null}</span>
                    {t.label}
                  </motion.div>
                ))}
              </div>
            </Card>

            {d.return_request && <div className="summary">Return requested: paid by the <strong>{d.return_request.paid_by}</strong>{d.return_request.received_at ? ", received" : ", not yet received"}.</div>}
            {d.decision && (
              <div className="summary">
                Decided: <strong>{words(d.decision.decision)}</strong>
                {d.decision.refund_kobo !== null && <> — refunded <Money kobo={d.decision.refund_kobo} /></>} · {dateTime(d.decision.decided_at)}
              </div>
            )}

            {isOpen && (
              <div className="row">
                <Button icon="undo" onClick={() => setMode("return")} disabled={d.status === "awaiting_return"}>
                  Ask for the item back
                </Button>
                {owner && (
                  <Button variant="primary" icon="dispute" disabled={sellerClock || d.status === "awaiting_return"} onClick={() => setMode("decide")}>
                    Decide
                  </Button>
                )}
                {owner && sellerClock && <span className="hint">The seller still has time to respond.</span>}
                {owner && d.status === "awaiting_return" && <span className="hint">Waiting for the return before deciding.</span>}
                {!owner && <span className="hint">Only the Owner decides.</span>}
              </div>
            )}

            <Card>
              <h2>Order chat</h2>
              {s.chat.length === 0 ? (
                <p className="muted">They never wrote to each other.</p>
              ) : (
                <div className="chat" aria-label="Chat transcript">
                  {s.chat.map((m) => (
                    <motion.div key={m.seq} className={`bubble ${m.from}`} initial={{ opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1 }}>
                      <small>{m.from} · {dateTime(m.sent_at)}</small>
                      {m.body}
                    </motion.div>
                  ))}
                </div>
              )}
              {s.flagged && s.flagged.length > 0 && (
                <div style={{ marginTop: 12, display: "grid", gap: 8 }}>
                  <strong>Messages we blocked or flagged (Owner only)</strong>
                  {s.flagged.map((f, i) => (
                    <div key={i} className="flagged">
                      <div className="row"><Badge tone="bad">{f.severity}</Badge><span className="muted">{f.from} · {words(f.category)} · {dateTime(f.at)}</span></div>
                      <div>{f.original}</div>
                    </div>
                  ))}
                </div>
              )}
              <p className="hint">Opening this screen is written to the audit log.</p>
            </Card>

            {s.delivery_events.length > 0 && (
              <Card>
                <h2>Delivery events</h2>
                <ol className="timeline" style={{ marginTop: 8 }}>
                  {s.delivery_events.map((e, i) => (
                    <li key={i}><span className="dot" /><strong>{words(e.status)}</strong> <span className="muted">· {e.source} · {dateTime(e.occurred_at ?? e.received_at)}</span></li>
                  ))}
                </ol>
              </Card>
            )}
          </>
        )}
      </Drawer>
      {s && d && <ReturnModal open={mode === "return"} id={d.id} onClose={() => setMode(null)} />}
      {s && d && owner && <DecideModal open={mode === "decide"} screen={s} onClose={() => setMode(null)} />}
    </>
  );
}

function ReturnModal({ open, id, onClose }: { open: boolean; id: string; onClose: () => void }) {
  const [paidBy, setPaidBy] = useState<"buyer" | "seller">("buyer");
  const [note, setNote] = useState("");
  const act = useAction({ run: () => api.post(`/admin/disputes/${id}/request-return`, { paid_by: paidBy, note: note.trim() }), success: "Return requested.", invalidate: [["dispute", id], ["disputes"]] });
  return (
    <Modal
      open={open}
      title="Ask for the item back"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" icon="undo" loading={act.busy} disabled={!note.trim()} onClick={() => void act.run(undefined).then(onClose).catch(() => {})}>Request return</Button>
        </>
      }
    >
      <p className="muted">Nobody moves money for return shipping; they arrange it. Say who is at fault so it is on record.</p>
      <fieldset style={{ border: 0, padding: 0, marginTop: 12, display: "grid", gap: 8 }}>
        {(["buyer", "seller"] as const).map((w) => (
          <label key={w} className={`choice ${paidBy === w ? "on" : ""}`}>
            <input type="radio" name="paid_by" checked={paidBy === w} onChange={() => setPaidBy(w)} />
            <span><strong>The {w} pays for the return</strong><br /><span className="muted">{w === "buyer" ? "The buyer is at fault, or changed their mind." : "The seller is at fault (wrong or damaged item)."}</span></span>
          </label>
        ))}
      </fieldset>
      <label className="field"><span>Note (kept in the audit log)</span><textarea rows={3} value={note} onChange={(e) => setNote(e.target.value)} maxLength={500} /></label>
    </Modal>
  );
}

type Decision = "full_refund" | "partial_refund" | "release";

/** The Owner's decision, with the money laid out before they commit. */
export function DecideModal({ open, screen, onClose }: { open: boolean; screen: DisputeScreen; onClose: () => void }) {
  const { order, dispute } = screen;
  const [decision, setDecision] = useState<Decision>("full_refund");
  const [amount, setAmount] = useState("");
  const [note, setNote] = useState("");
  const [confirm, setConfirm] = useState(false);
  const itemsKobo = order.items_total_kobo;
  const refundKobo = Math.round(Number(amount) * 100);
  const partialOk = Number.isFinite(refundKobo) && refundKobo >= 1 && refundKobo < itemsKobo;
  const valid = note.trim().length > 0 && (decision !== "partial_refund" || partialOk);

  useEffect(() => {
    if (open) {
      setDecision("full_refund");
      setAmount("");
      setNote("");
      setConfirm(false);
    }
  }, [open]);

  const act = useAction({
    run: () => api.post(`/admin/disputes/${dispute.id}/decide`, { decision, refund_kobo: decision === "partial_refund" ? refundKobo : undefined, note: note.trim() }),
    success: "Decision recorded. Both sides are told.",
    invalidate: [["dispute", dispute.id], ["disputes"], ["reconciliation"]],
  });

  const outcome =
    decision === "full_refund" ? (
      <>The buyer gets <strong>{formatKobo(itemsKobo + order.delivery_fee_kobo)}</strong> back (items and delivery). The seller is paid nothing.</>
    ) : decision === "release" ? (
      <>The seller is paid: items {formatKobo(itemsKobo)} less the market's cut. The buyer gets nothing back.</>
    ) : partialOk ? (
      <>The buyer gets <strong>{formatKobo(refundKobo)}</strong> back. The seller is paid on the other <strong>{formatKobo(itemsKobo - refundKobo)}</strong>, less the market's cut. Delivery is not refunded.</>
    ) : (
      <>Enter an amount between ₦0.01 and {formatKobo(itemsKobo - 1)}.</>
    );

  return (
    <Modal
      open={open}
      title="Decide this dispute"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          {!confirm ? (
            <Button variant="primary" disabled={!valid} onClick={() => setConfirm(true)}>Review</Button>
          ) : (
            <Button variant={decision === "release" ? "primary" : "danger"} icon="check" loading={act.busy} onClick={() => void act.run(undefined).then(onClose).catch(() => setConfirm(false))}>
              Confirm and pay out
            </Button>
          )}
        </>
      }
    >
      {!confirm ? (
        <>
          <fieldset style={{ border: 0, padding: 0, display: "grid", gap: 8 }}>
            {([
              ["full_refund", "Refund the buyer in full", "Items and delivery go back. The seller gets nothing."],
              ["partial_refund", "Refund part of it", "Part of the item total goes back; the rest is released to the seller."],
              ["release", "Release to the seller", "The complaint is not upheld."],
            ] as [Decision, string, string][]).map(([v, t, h]) => (
              <label key={v} className={`choice ${decision === v ? "on" : ""}`}>
                <input type="radio" name="decision" checked={decision === v} onChange={() => setDecision(v)} />
                <span><strong>{t}</strong><br /><span className="muted">{h}</span></span>
              </label>
            ))}
          </fieldset>
          {decision === "partial_refund" && (
            <label className="field">
              <span>Refund amount (₦, up to the item total of {formatKobo(itemsKobo)})</span>
              <input inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} aria-invalid={amount !== "" && !partialOk} />
            </label>
          )}
          <motion.div className="summary" style={{ marginTop: 14 }} key={`${decision}-${partialOk}`} initial={{ opacity: 0.4, y: 4 }} animate={{ opacity: 1, y: 0 }}>{outcome}</motion.div>
          <label className="field"><span>Reasoning (kept in the audit log)</span><textarea rows={3} value={note} onChange={(e) => setNote(e.target.value)} maxLength={500} /></label>
        </>
      ) : (
        <>
          <div className="summary">{outcome}</div>
          <p className="muted" style={{ marginTop: 12 }}>This moves real money and cannot be undone. Both the buyer and the seller are notified.</p>
          <p style={{ marginTop: 8 }}>Your reasoning: “{note.trim()}”</p>
        </>
      )}
    </Modal>
  );
}
