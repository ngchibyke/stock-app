import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useAction } from "../hooks";
import { Button } from "../ui/Button";
import { Badge, Card, ErrorNote, Toggle } from "../ui/kit";
import { SkeletonForm } from "../ui/Skeleton";

export interface PartnerStatus {
  configured: boolean;
  enabled: boolean;
  /** null when the partner was not asked (a settings change makes no call). */
  connected: boolean | null;
  breaker: "closed" | "open" | "half_open";
  features: { name: string; on: boolean }[];
  monthly_call_budget: number | null;
  calls_this_month: number;
  usage: { feature: string; calls: number; errors: number }[];
  last_error: string | null;
}

const FEATURE_NAME = /^[a-z0-9_]{1,40}$/;

/** Plain words for a fixed error code. The partner's own words are never shown. */
export function errorWords(code: string | null): string | null {
  switch (code) {
    case null:
      return null;
    case "unavailable":
      return "The partner did not answer in time or could not be reached.";
    case "rejected":
      return "The partner refused the last request.";
    case "bad_response":
      return "The partner's last answer was too large or not readable.";
    case "circuit_open":
      return "Calls are paused after repeated failures.";
    case "budget_spent":
      return "This month's call budget is spent.";
    case "blocked":
      return "A request was stopped because it carried a field that may not be sent.";
    default:
      return "The last call failed.";
  }
}

function breakerWords(b: PartnerStatus["breaker"]): string {
  if (b === "open") return "Paused: the partner failed several times in a row. One test call goes out after 30 seconds.";
  if (b === "half_open") return "Testing: the next call decides whether calls resume.";
  return "Calls are going through normally.";
}

/** The Owner's view of the partner service, and its switches. Every change needs a reason. */
export function PartnerCard() {
  const q = useQuery({ queryKey: ["partner"], queryFn: () => api.get<PartnerStatus>("/admin/partner/status") });
  const [enabled, setEnabled] = useState(false);
  const [flags, setFlags] = useState<Record<string, boolean>>({});
  const [budget, setBudget] = useState("");
  const [note, setNote] = useState("");
  const [newFeature, setNewFeature] = useState("");

  // Fill the form from the server once, and again after a save.
  useEffect(() => {
    if (!q.data) return;
    setEnabled(q.data.enabled);
    setFlags(Object.fromEntries(q.data.features.map((f) => [f.name, f.on])));
    setBudget(q.data.monthly_call_budget === null ? "" : String(q.data.monthly_call_budget));
    setNote("");
    setNewFeature("");
  }, [q.data]);

  const s = q.data;
  const budgetValue = budget.trim() === "" ? null : /^\d{1,9}$/.test(budget.trim()) ? Number(budget.trim()) : undefined;
  const featureOk = newFeature === "" || FEATURE_NAME.test(newFeature);

  const changes: Record<string, unknown> = {};
  if (s) {
    if (enabled !== s.enabled) changes.enabled = enabled;
    const changed: Record<string, boolean> = {};
    for (const [name, on] of Object.entries(flags)) {
      if (s.features.find((f) => f.name === name)?.on !== on) changed[name] = on;
    }
    if (newFeature !== "" && featureOk) changed[newFeature] = true;
    if (Object.keys(changed).length > 0) changes.features = changed;
    if (budgetValue !== undefined && budgetValue !== s.monthly_call_budget) changes.monthly_call_budget = budgetValue;
  }
  const count = Object.keys(changes).length;
  const ready = count > 0 && note.trim() !== "" && budgetValue !== undefined && featureOk;

  const save = useAction({
    run: () => api.patch<PartnerStatus>("/admin/partner", { ...changes, note: note.trim() }),
    success: "Partner settings saved.",
    invalidate: [["partner"]],
  });

  return (
    <Card>
      <h2>Partner service</h2>
      <p className="muted">
        A service built by your other company, called through one safe door. The marketplace works the same with it off, down or over budget.
      </p>
      {q.isPending && <SkeletonForm fields={3} />}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {s && !s.configured && (
        <p className="muted" role="status">
          No partner is connected. To connect one, the server needs PARTNER_API_BASE_URL, PARTNER_API_KEY and PARTNER_WEBHOOK_SECRET.
        </p>
      )}
      {s && s.configured && (
        <div className="stack">
          <div className="row">
            <strong>Connection</strong>
            {s.connected === true && <Badge tone="good">Connected</Badge>}
            {s.connected === false && <Badge tone={s.enabled ? "bad" : "neutral"}>{s.enabled ? "Not reachable" : "Switched off"}</Badge>}
            {s.connected === null && <Badge>Not checked</Badge>}
          </div>
          <p className="muted">{breakerWords(s.breaker)}</p>
          {errorWords(s.last_error) && (
            <p className="muted" role="status">
              Last problem: {errorWords(s.last_error)}
            </p>
          )}

          <div className="field">
            <span>Use the partner</span>
            <div className="row">
              <Toggle checked={enabled} onChange={setEnabled} label="Use the partner" />
              <span className="muted">{enabled ? "On" : "Off"}</span>
            </div>
          </div>

          <div className="field">
            <span>Features</span>
            {Object.keys(flags).length === 0 && <span className="hint">None yet. A feature is named here when the partner offers one.</span>}
            {Object.entries(flags).map(([name, on]) => (
              <div className="row" key={name}>
                <Toggle checked={on} onChange={(v) => setFlags((f) => ({ ...f, [name]: v }))} label={`Feature ${name}`} />
                <span>{name}</span>
              </div>
            ))}
            <label htmlFor="partner-new-feature" className="hint">
              Add a feature (lowercase letters, digits and _)
            </label>
            <input id="partner-new-feature" value={newFeature} onChange={(e) => setNewFeature(e.target.value)} aria-invalid={!featureOk || undefined} />
          </div>

          <div className="field">
            <label htmlFor="partner-budget">Monthly call budget</label>
            <input id="partner-budget" inputMode="numeric" value={budget} onChange={(e) => setBudget(e.target.value)} aria-invalid={budgetValue === undefined || undefined} />
            <span className="hint">Leave empty for no cap. Calls stop, and an alert is raised, when it is spent.</span>
          </div>

          <div className="field">
            <span>This month</span>
            <strong>
              {s.calls_this_month} {s.calls_this_month === 1 ? "call" : "calls"}
              {s.monthly_call_budget !== null ? ` of ${s.monthly_call_budget}` : " (no cap)"}
            </strong>
            {s.monthly_call_budget !== null && (
              <progress max={Math.max(s.monthly_call_budget, 1)} value={Math.min(s.calls_this_month, Math.max(s.monthly_call_budget, 1))} aria-label="Calls used this month" />
            )}
            {s.usage.length === 0 ? (
              <span className="hint">No calls yet this month.</span>
            ) : (
              <ul className="plain">
                {s.usage.map((u) => (
                  <li key={u.feature}>
                    {u.feature}: {u.calls} {u.calls === 1 ? "call" : "calls"}, {u.errors} {u.errors === 1 ? "error" : "errors"}
                  </li>
                ))}
              </ul>
            )}
          </div>

          <div className="field">
            <label htmlFor="partner-note">Reason for the change (required)</label>
            <input id="partner-note" value={note} maxLength={500} onChange={(e) => setNote(e.target.value)} />
            <span className="hint">Kept in the audit log with your name.</span>
          </div>
          <div className="row">
            <Button variant="primary" icon="check" loading={save.busy} done={save.done} disabled={!ready} onClick={() => void save.run(undefined).catch(() => undefined)}>
              Save partner settings
            </Button>
            {count > 0 && note.trim() === "" && <span className="hint">Add a reason to save.</span>}
          </div>
        </div>
      )}
    </Card>
  );
}
