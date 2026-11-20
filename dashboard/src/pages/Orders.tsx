import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { Link } from "react-router-dom";
import { api, formatKobo } from "../api";
import { dateTime, words } from "../format";
import { useAction } from "../hooks";
import { spring } from "../motion";
import { Button } from "../ui/Button";
import { Drawer } from "../ui/Drawer";
import { Icon } from "../ui/Icon";
import { ConfirmDialog } from "../ui/Modal";
import { DataTable, LoadMore } from "../ui/Table";
import { Badge, Card, CopyChip, EmptyState, ErrorNote, Money, StatusBadge } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonDetail, SkeletonTable } from "../ui/Skeleton";

export interface OrderRow {
  order_id: string;
  store_id: string;
  status: string;
  items_total_kobo: number;
  delivery_fee_kobo: number;
  held: boolean;
  paid_at: string | null;
  created_at: string;
}
interface OrderPage {
  items: OrderRow[];
  next_cursor: string | null;
}
export interface LedgerPosting {
  txn_id: string;
  kind: string | null;
  ref: string | null;
  posted_at: string | null;
  entries: { account: string; amount_kobo: number; kind: string }[];
  sum_kobo: number;
}
export interface OrderDetail {
  order: OrderRow & {
    platform_cut_bps: number;
    platform_cut_kobo: number;
    collection_fee_share_kobo: number;
    hold_reason: string | null;
    delivered_at: string | null;
    completed_at: string | null;
    cancelled_at: string | null;
  };
  items: { product_name: string; attributes: Record<string, unknown>; unit_price_kobo: number; quantity: number }[];
  payment: { reference: string; method: string; provider: string; status: string; provider_flagged: boolean };
  delivery_events: { source: string; status: string; occurred_at: string | null; received_at: string }[];
  refund: { id: string; kind: string; amount_kobo: number; status: string } | null;
  payout: { id: string; status: string; amount_kobo: number; fee_kobo: number } | null;
  dispute: { id: string; status: string } | null;
  ledger: { postings: LedgerPosting[]; balances: { account: string; kobo: number }[]; balance_kobo: number };
}

const STATUSES = ["", "paid", "picked_up", "out_for_delivery", "delivered", "completed", "refund_pending", "refunded", "cancelled"] as const;
const RELEASABLE = ["paid", "picked_up", "out_for_delivery", "delivered"];
const OPEN_DISPUTE = ["awaiting_seller", "awaiting_return", "awaiting_decision"];

export function Orders() {
  const [status, setStatus] = useState<(typeof STATUSES)[number]>("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [open, setOpen] = useState<string | null>(null);
  const list = useInfiniteQuery({
    queryKey: ["orders", status, from, to],
    initialPageParam: "",
    queryFn: ({ pageParam }) =>
      api.get<OrderPage>("/admin/orders", {
        status,
        before: pageParam,
        limit: 25,
        from: from ? new Date(`${from}T00:00:00`).toISOString() : undefined,
        to: to ? new Date(`${to}T23:59:59`).toISOString() : undefined,
      }),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
  });
  const rows = list.data?.pages.flatMap((p) => p.items) ?? [];

  return (
    <>
      <PageHeader title="Orders" subtitle="Every order, its money, and the ledger behind it." />
      <div className="toolbar">
        <select aria-label="Status" value={status} onChange={(e) => setStatus(e.target.value as typeof status)} style={{ width: "auto" }}>
          {STATUSES.map((s) => (
            <option key={s} value={s}>
              {s === "" ? "Any status" : words(s)}
            </option>
          ))}
        </select>
        <label className="row muted">
          From <input type="date" value={from} onChange={(e) => setFrom(e.target.value)} style={{ width: "auto" }} />
        </label>
        <label className="row muted">
          To <input type="date" value={to} onChange={(e) => setTo(e.target.value)} style={{ width: "auto" }} />
        </label>
      </div>
      <Card>
        {list.isPending && <SkeletonTable rows={8} cols={5} label="Loading orders" />}
        {list.isError && <ErrorNote error={list.error} onRetry={() => void list.refetch()} />}
        {list.data && rows.length === 0 && <EmptyState icon="orders" title="No orders match" hint="Change the status or the dates." />}
        {rows.length > 0 && (
          <DataTable
            label="Orders"
            rows={rows}
            rowKey={(o) => o.order_id}
            onRowClick={(o) => setOpen(o.order_id)}
            columns={[
              { key: "id", header: "Order", cell: (o) => <span className="mono">{o.order_id.slice(0, 8)}</span> },
              { key: "status", header: "Status", cell: (o) => <span className="row"><StatusBadge status={o.status} />{o.held && <Badge tone="warn">held</Badge>}</span> },
              { key: "items", header: "Items", cell: (o) => <Money kobo={o.items_total_kobo} /> },
              { key: "del", header: "Delivery", cell: (o) => <Money kobo={o.delivery_fee_kobo} /> },
              { key: "created", header: "Placed", cell: (o) => dateTime(o.created_at) },
            ]}
          />
        )}
        <LoadMore hasMore={!!list.hasNextPage} loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()} />
      </Card>
      <OrderDrawer id={open} onClose={() => setOpen(null)} />
    </>
  );
}

type Act = "release" | "refund" | "hold" | "unhold";

function OrderDrawer({ id, onClose }: { id: string | null; onClose: () => void }) {
  const q = useQuery({ queryKey: ["order", id], queryFn: () => api.get<OrderDetail>(`/admin/orders/${id}`), enabled: id !== null });
  const [act, setAct] = useState<Act | null>(null);
  const refresh = [["order", id], ["orders"], ["reconciliation"]];
  const doAct = useAction({
    run: ({ kind, text }: { kind: Act; text: string }) => api.post(`/admin/orders/${id}/${kind}`, kind === "hold" ? { reason: text } : { note: text }),
    success: "Done. It is in the audit log.",
    invalidate: refresh,
  });
  const d = q.data;
  const o = d?.order;
  const disputeOpen = !!d?.dispute && OPEN_DISPUTE.includes(d.dispute.status);
  const sellerGets = o ? o.items_total_kobo - o.platform_cut_kobo : 0;

  const copy: Record<Act, { title: string; summary: React.ReactNode; label: string; danger?: boolean }> = {
    release: {
      title: "Release to the seller?",
      label: "Release now",
      summary: o && (
        <>
          Pay out this order now. Items <Money kobo={o.items_total_kobo} />, less the market's cut <Money kobo={o.platform_cut_kobo} /> ({o.platform_cut_bps / 100}%), leaves the seller{" "}
          <strong><Money kobo={sellerGets} /></strong> before payment fees.
        </>
      ),
    },
    refund: {
      title: "Refund the buyer in full?",
      label: "Refund in full",
      danger: true,
      summary: o && (
        <>
          Return <strong><Money kobo={o.items_total_kobo + o.delivery_fee_kobo} /></strong> (items and delivery) to the buyer. The seller is not paid. The market absorbs the collection fee.
        </>
      ),
    },
    hold: { title: "Hold this order?", label: "Hold", summary: <>A held order never auto-releases, and no payout is sent, until you lift the hold.</> },
    unhold: { title: "Lift the hold?", label: "Lift hold", summary: <>The order goes back to its normal timers.</> },
  };

  return (
    <>
      <Drawer open={id !== null} title={o ? `Order ${o.order_id.slice(0, 8)}` : "Order"} onClose={onClose}>
        {q.isPending && <SkeletonDetail rows={8} label="Loading the order" />}
        {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
        {d && o && (
          <>
            <div className="row">
              <StatusBadge status={o.status} />
              {o.held && <Badge tone="warn">held{o.hold_reason ? `: ${o.hold_reason}` : ""}</Badge>}
              {d.payment.provider_flagged && <Badge tone="bad">payment flagged</Badge>}
              <CopyChip value={o.order_id} label="copy id" />
            </div>

            <Card>
              <dl className="kv">
                <dt>Items</dt><dd><Money kobo={o.items_total_kobo} /></dd>
                <dt>Delivery</dt><dd><Money kobo={o.delivery_fee_kobo} /></dd>
                <dt>Platform cut</dt><dd><Money kobo={o.platform_cut_kobo} /> <span className="muted">({o.platform_cut_bps / 100}%)</span></dd>
                <dt>Placed</dt><dd>{dateTime(o.created_at)}</dd>
                <dt>Paid</dt><dd>{dateTime(o.paid_at)}</dd>
                <dt>Delivered</dt><dd>{dateTime(o.delivered_at)}</dd>
                <dt>Payment</dt><dd className="mono">{d.payment.reference} · {d.payment.provider}</dd>
                {d.payout && (<><dt>Payout</dt><dd><StatusBadge status={d.payout.status} /> <Money kobo={d.payout.amount_kobo} /></dd></>)}
                {d.refund && (<><dt>Refund</dt><dd><StatusBadge status={d.refund.status} /> {d.refund.kind} <Money kobo={d.refund.amount_kobo} /></dd></>)}
              </dl>
              <table className="table" style={{ marginTop: 10 }} aria-label="Items">
                <tbody>
                  {d.items.map((i, n) => (
                    <tr key={n}>
                      <td>{i.product_name}</td>
                      <td className="muted">× {i.quantity}</td>
                      <td style={{ textAlign: "right" }}><Money kobo={i.unit_price_kobo * i.quantity} /></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Card>

            {d.dispute && (
              <div className="summary row" style={{ justifyContent: "space-between" }}>
                <span><Icon name="dispute" size={18} /> Dispute <StatusBadge status={d.dispute.status} /></span>
                <Link to="/disputes" className="link">Open disputes</Link>
              </div>
            )}

            <div className="row">
              {disputeOpen ? (
                <span className="hint">An open dispute decides the money. Use the dispute screen.</span>
              ) : (
                <>
                  {RELEASABLE.includes(o.status) && !o.held && <Button variant="primary" icon="check" onClick={() => setAct("release")}>Release</Button>}
                  {o.held && ["paid", "picked_up", "out_for_delivery", "delivered"].includes(o.status) && <Button variant="danger" icon="undo" onClick={() => setAct("refund")}>Refund</Button>}
                  {o.held ? <Button icon="undo" onClick={() => setAct("unhold")}>Lift hold</Button> : ["paid", "picked_up", "out_for_delivery", "delivered", "completed"].includes(o.status) && <Button icon="clock" onClick={() => setAct("hold")}>Hold</Button>}
                </>
              )}
            </div>
            {!o.held && !disputeOpen && RELEASABLE.includes(o.status) && <p className="hint">To refund, hold the order first.</p>}

            <h2>Delivery</h2>
            <ol className="timeline">
              {d.delivery_events.length === 0 && <li className="muted">No delivery events yet.</li>}
              {d.delivery_events.map((e, i) => (
                <motion.li key={i} initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={{ delay: i * 0.04 }}>
                  <span className="dot" />
                  <strong>{words(e.status)}</strong> <span className="muted">· {e.source} · {dateTime(e.occurred_at ?? e.received_at)}</span>
                </motion.li>
              ))}
            </ol>

            <Ledger ledger={d.ledger} />
          </>
        )}
      </Drawer>
      {act && (
        <ConfirmDialog
          open
          title={copy[act].title}
          summary={copy[act].summary}
          confirmLabel={copy[act].label}
          danger={copy[act].danger}
          noteLabel={act === "hold" ? "Reason for the hold" : undefined}
          onClose={() => setAct(null)}
          onConfirm={async (text) => {
            await doAct.run({ kind: act, text });
          }}
        />
      )}
    </>
  );
}

/** The ledger as postings that open to show their entries; every one must balance. */
export function Ledger({ ledger }: { ledger: OrderDetail["ledger"] }) {
  const [openKey, setOpenKey] = useState<string | null>(null);
  return (
    <>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <h2>Ledger</h2>
        <Badge tone={ledger.balance_kobo === 0 ? "good" : "bad"}>{ledger.balance_kobo === 0 ? "balanced" : `off by ${formatKobo(ledger.balance_kobo)}`}</Badge>
      </div>
      {ledger.postings.length === 0 && <p className="muted">Nothing posted yet.</p>}
      {ledger.postings.map((p) => {
        const isOpen = openKey === p.txn_id;
        return (
          <motion.div key={p.txn_id} layout className="posting" transition={spring.snappy}>
            <button type="button" className="posting-head" aria-expanded={isOpen} onClick={() => setOpenKey(isOpen ? null : p.txn_id)}>
              <Icon name={p.sum_kobo === 0 ? "check" : "warn"} size={16} interactive={false} />
              <strong>{words(p.kind ?? "entry")}</strong>
              <span className="muted">{p.posted_at ? dateTime(p.posted_at) : ""}</span>
              <motion.span animate={{ rotate: isOpen ? 90 : 0 }} style={{ marginLeft: "auto", display: "inline-flex" }}>
                <Icon name="chevron" size={16} interactive={false} />
              </motion.span>
            </button>
            <AnimatePresence initial={false}>
              {isOpen && (
                <motion.table className="table" aria-label="Entries" initial={{ height: 0, opacity: 0 }} animate={{ height: "auto", opacity: 1 }} exit={{ height: 0, opacity: 0 }} style={{ overflow: "hidden" }}>
                  <tbody>
                    {p.entries.map((e, i) => (
                      <tr key={i}>
                        <td>{words(e.account)}</td>
                        <td className="muted">{e.amount_kobo > 0 ? "debit" : "credit"}</td>
                        <td style={{ textAlign: "right" }}><Money kobo={Math.abs(e.amount_kobo)} /></td>
                      </tr>
                    ))}
                  </tbody>
                </motion.table>
              )}
            </AnimatePresence>
          </motion.div>
        );
      })}
      {ledger.balances.length > 0 && (
        <dl className="kv">
          {ledger.balances.map((b) => (
            <span key={b.account} style={{ display: "contents" }}>
              <dt>{words(b.account)}</dt>
              <dd><Money kobo={b.kobo} signed /></dd>
            </span>
          ))}
        </dl>
      )}
      <p className="hint">Positive is a debit. Day-to-day you only need "balanced".</p>
    </>
  );
}
