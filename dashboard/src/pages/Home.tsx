import { useQuery } from "@tanstack/react-query";
import { motion } from "motion/react";
import { Link } from "react-router-dom";
import { api } from "../api";
import { useAuth } from "../auth";
import { spring } from "../motion";
import { Card, Counter, ErrorNote, Item, Stagger } from "../ui/kit";
import { Icon, type IconName } from "../ui/Icon";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonStats } from "../ui/Skeleton";

interface Reconciliation {
  payouts: unknown[];
  refunds: unknown[];
  flagged_checkouts: unknown[];
  mismatched_checkouts: unknown[];
  late_provider_outcomes: unknown[];
  payout_amount_mismatches: unknown[];
  blocked_payouts: unknown[];
  shipments_attention: unknown[];
  orders_attention: unknown[];
  wallet_alerts: unknown[];
}

interface Tile {
  key: keyof Reconciliation;
  label: string;
  icon: IconName;
  /** Where the person goes to clear it. */
  to: string;
  /** How loud it is when it is not zero: money at risk is red, the rest amber. */
  tone: "bad" | "warn";
}

const tiles: Tile[] = [
  { key: "payouts", label: "Payouts needing attention", icon: "money", to: "/money", tone: "bad" },
  { key: "blocked_payouts", label: "Payouts blocked 7+ days", icon: "clock", to: "/money", tone: "warn" },
  { key: "refunds", label: "Refunds stuck", icon: "undo", to: "/money?tab=refunds", tone: "bad" },
  { key: "flagged_checkouts", label: "Flagged payments", icon: "flag", to: "/money?tab=payments", tone: "bad" },
  { key: "mismatched_checkouts", label: "Amount mismatches", icon: "warn", to: "/money?tab=payments", tone: "bad" },
  { key: "shipments_attention", label: "Shipments needing a person", icon: "truck", to: "/money?tab=shipments", tone: "warn" },
  { key: "orders_attention", label: "Orders needing a decision", icon: "orders", to: "/money?tab=orders", tone: "warn" },
  { key: "late_provider_outcomes", label: "Late provider outcomes", icon: "alert", to: "/alerts", tone: "warn" },
  { key: "payout_amount_mismatches", label: "Payout amount mismatches", icon: "money", to: "/alerts", tone: "bad" },
  { key: "wallet_alerts", label: "Courier wallet warnings", icon: "alert", to: "/alerts", tone: "warn" },
];

/** How many of the checks are clear, as a ring that fills in. */
function Ring({ clear, of }: { clear: number; of: number }) {
  const r = 34;
  const c = 2 * Math.PI * r;
  const allClear = clear === of;
  return (
    <svg viewBox="0 0 84 84" width="84" height="84" role="img" aria-label={`${clear} of ${of} checks clear`} className={`ring ${allClear ? "good" : ""}`}>
      <circle cx="42" cy="42" r={r} className="ring-track" />
      <motion.circle cx="42" cy="42" r={r} className="ring-fill" strokeDasharray={c} initial={{ strokeDashoffset: c }} animate={{ strokeDashoffset: c * (1 - clear / of) }} transition={{ duration: 0.9, ease: [0.22, 1, 0.36, 1] }} transform="rotate(-90 42 42)" />
      <text x="42" y="47" textAnchor="middle" className="ring-text">
        {clear}/{of}
      </text>
    </svg>
  );
}

export function Home() {
  const { user } = useAuth();
  const owner = user?.role === "owner";
  const rec = useQuery({
    queryKey: ["reconciliation"],
    queryFn: () => api.get<Reconciliation>("/admin/reconciliation"),
    refetchInterval: 60_000,
  });
  const counts = rec.data ? tiles.map((t) => rec.data[t.key].length) : [];
  const total = counts.reduce((n, c) => n + c, 0);
  const clear = counts.filter((c) => c === 0).length;
  // What needs a person first: anything that is not zero, then the calm ones.
  const ordered = rec.data ? tiles.map((t, i) => ({ t, n: counts[i] })).sort((a, b) => Number(b.n > 0) - Number(a.n > 0)) : [];

  return (
    <>
      <PageHeader
        title={`Good to see you, ${user?.display_name.split(" ")[0]}`}
        subtitle={owner ? "Everything waiting on you, oldest first." : "Your desk: disputes and moderation."}
      />
      {rec.isPending && <SkeletonStats count={8} label="Loading what is waiting on you" />}
      {rec.isError && <ErrorNote error={rec.error} onRetry={() => void rec.refetch()} />}
      {rec.data && (
        <div className="stack">
          <Card className={`hero ${total === 0 ? "hero-clear" : ""}`} hover>
            <div className="row" style={{ justifyContent: "space-between", flexWrap: "nowrap" }}>
              <div>
                <div className="muted" style={{ fontWeight: 600 }}>
                  Open items
                </div>
                <div style={{ fontSize: 40, fontWeight: 700, letterSpacing: "-0.03em", lineHeight: 1.15 }}>
                  <Counter to={total} />
                </div>
                <div className="muted" style={{ marginTop: 4 }}>
                  {total === 0 ? "Nothing is waiting. The market is running clean." : `${tiles.length - clear} of ${tiles.length} checks need a person. Start with the highlighted ones.`}
                </div>
              </div>
              <Ring clear={clear} of={tiles.length} />
            </div>
          </Card>
          <Stagger className="grid" >
            {ordered.map(({ t, n }) => (
              <Item key={t.key}>
                <motion.div className={`card tile ${n > 0 ? `tile-${t.tone}` : "tile-calm"}`} initial="rest" whileHover="hover" whileTap="tap" variants={{ rest: { y: 0 }, hover: { y: -3 }, tap: { y: 0, scale: 0.985 } }} transition={spring.snappy}>
                  <span className="stat">
                    <span className="label">
                      <span className="tile-icon">
                        <Icon name={n > 0 ? t.icon : "check"} size={16} interactive={false} />
                      </span>
                      {t.label}
                    </span>
                    <span className="value">
                      <Counter to={n} />
                    </span>
                  </span>
                  {owner ? (
                    <Link to={t.to} className="tile-link" aria-label={`${t.label}: ${n}. ${n > 0 ? "Open to clear them" : "Nothing to do"}`}>
                      <span className="tile-go">
                        <Icon name="arrow" size={18} interactive={false} />
                      </span>
                    </Link>
                  ) : null}
                </motion.div>
              </Item>
            ))}
          </Stagger>
        </div>
      )}
    </>
  );
}
