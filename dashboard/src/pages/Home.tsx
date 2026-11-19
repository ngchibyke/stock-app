import { useQuery } from "@tanstack/react-query";
import { api } from "../api";
import { useAuth } from "../auth";
import { Card, Counter, ErrorNote, Item, Skeleton, Stagger } from "../ui/kit";
import { Icon, type IconName } from "../ui/Icon";

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

const tiles: { key: keyof Reconciliation; label: string; icon: IconName }[] = [
  { key: "payouts", label: "Payouts needing attention", icon: "money" },
  { key: "blocked_payouts", label: "Payouts blocked 7+ days", icon: "clock" },
  { key: "refunds", label: "Refunds stuck", icon: "undo" },
  { key: "flagged_checkouts", label: "Flagged payments", icon: "flag" },
  { key: "mismatched_checkouts", label: "Amount mismatches", icon: "warn" },
  { key: "shipments_attention", label: "Shipments needing a person", icon: "truck" },
  { key: "orders_attention", label: "Orders needing a decision", icon: "orders" },
  { key: "late_provider_outcomes", label: "Late provider outcomes", icon: "alert" },
  { key: "payout_amount_mismatches", label: "Payout amount mismatches", icon: "money" },
  { key: "wallet_alerts", label: "Courier wallet warnings", icon: "alert" },
];

export function Home() {
  const { user } = useAuth();
  const owner = user?.role === "owner";
  const rec = useQuery({
    queryKey: ["reconciliation"],
    queryFn: () => api.get<Reconciliation>("/admin/reconciliation"),
    refetchInterval: 60_000,
  });
  const total = rec.data ? tiles.reduce((n, t) => n + rec.data[t.key].length, 0) : 0;

  return (
    <>
      <div className="page-head">
        <div>
          <h1>Good to see you, {user?.display_name.split(" ")[0]}</h1>
          <p>{owner ? "Everything waiting on you, oldest first." : "Your desk: disputes and moderation."}</p>
        </div>
      </div>
      {rec.isPending && (
        <div className="grid">
          {tiles.slice(0, 4).map((t) => (
            <Card key={t.key}>
              <Skeleton lines={2} />
            </Card>
          ))}
        </div>
      )}
      {rec.isError && <ErrorNote error={rec.error} onRetry={() => void rec.refetch()} />}
      {rec.data && (
        <>
          <Card className="hero" hover>
            <div className="row" style={{ justifyContent: "space-between" }}>
              <div>
                <div className="muted" style={{ fontWeight: 600 }}>
                  Open items
                </div>
                <div style={{ fontSize: 40, fontWeight: 700, letterSpacing: "-0.03em" }}>
                  <Counter to={total} />
                </div>
              </div>
              <Icon name={total === 0 ? "check" : "alert"} size={40} active />
            </div>
            <div className="muted">{total === 0 ? "Nothing is waiting. The market is running clean." : "Open the sections below to clear them."}</div>
          </Card>
          <Stagger className="grid">
            {tiles.map((t) => (
              <Item key={t.key}>
                <Card hover>
                  <div className="stat">
                    <span className="label">
                      <Icon name={t.icon} size={18} />
                      {t.label}
                    </span>
                    <span className="value">
                      <Counter to={rec.data[t.key].length} />
                    </span>
                  </div>
                </Card>
              </Item>
            ))}
          </Stagger>
        </>
      )}
    </>
  );
}
