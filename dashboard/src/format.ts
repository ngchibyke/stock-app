export const day = (iso: string | null | undefined) =>
  iso ? new Date(iso).toLocaleDateString("en-NG", { day: "numeric", month: "short", year: "numeric" }) : "–";

export const dateTime = (iso: string | null | undefined) =>
  iso ? new Date(iso).toLocaleString("en-NG", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "–";

/** "2 hours ago", "3 days ago": for queues where age is what matters. */
export function ago(iso: string, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - Date.parse(iso)) / 1000));
  if (s < 60) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} hr ago`;
  return `${Math.round(h / 24)} days ago`;
}

export const words = (s: string) => s.replace(/[._]/g, " ").replace(/\s+/g, " ").trim();

/** Alert kinds from the server, in plain words. */
export const ALERT_LABELS: Record<string, { title: string; hint: string }> = {
  "payout.provider_outcome_after_close": {
    title: "Late answer on a closed payout",
    hint: "The provider reported an outcome on a payout you already settled by hand. A success there may be a double payment.",
  },
  "payout.amount_mismatch": {
    title: "Payout amount differs",
    hint: "The provider says it paid a different amount than we sent. The payout is settled at our amount; chase the difference.",
  },
  "payout.reversed_after_success": {
    title: "A paid payout was reversed",
    hint: "The bank reversed a transfer that had succeeded. The ledger is corrected and the payout needs you.",
  },
  "courier.wallet_alert": {
    title: "Courier wallet warning",
    hint: "The courier wallet may be low. Fund it, then re-book any failed shipments.",
  },
  "statement.mismatch": {
    title: "Provider statement does not match",
    hint: "A day's statement differs from the ledger by more than the tolerance.",
  },
  "identity.budget_exhausted": {
    title: "Identity checks paused",
    hint: "Today's budget for identity provider calls is used up, so sellers cannot start a check. Raise the budget in settings, or wait for the day to roll over.",
  },
  "order.no_delivery_method": {
    title: "Paid order with no delivery method",
    hint: "An old order the ship-deadline job cannot judge. Decide whether to release or refund it.",
  },
  "order.quote_reused": {
    title: "Courier quote reused",
    hint: "A late payment revived an order whose courier quote another order also holds, so one token may book twice.",
  },
  "order.repeated_handover_lockout": {
    title: "Handover code locked out repeatedly",
    hint: "Someone keeps entering wrong delivery codes on this order. Check with the seller and buyer.",
  },
};
