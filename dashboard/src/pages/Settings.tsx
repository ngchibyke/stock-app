import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useMemo, useState } from "react";
import { ApiError, api, formatKobo } from "../api";
import { useAction } from "../hooks";
import { spring } from "../motion";
import { Button } from "../ui/Button";
import { Card, ErrorNote, Toggle } from "../ui/kit";
import { PageHeader } from "../ui/PageHeader";
import { SkeletonForm } from "../ui/Skeleton";
import { Banner } from "../ui/Controls";
import { PartnerCard } from "./Partner";

export interface Settings {
  platform_cut_bps: number;
  fee_small_kobo: number;
  fee_large_kobo: number;
  fee_threshold_kobo: number;
  stamp_duty_kobo: number;
  stamp_duty_threshold_kobo: number;
  enabled_methods: string[];
  payout_provider: string;
  checkout_ttl_minutes: number;
  ship_deadline_hours: number;
  release_after_hours: number;
  backstop_days: number;
  payout_settlement_buffer_hours: number;
  jobs_ship_deadline_enabled: boolean;
  delivery_cutoff_minutes: number;
  courier_quote_ttl_minutes: number;
  courier_price_tolerance_bps: number;
  strike_window_days: number;
  mute_after_strikes: number;
  mute_hours: number;
  owner_review_after_strikes: number;
  updated_at: string;
}

type Key = Exclude<keyof Settings, "enabled_methods" | "payout_provider" | "updated_at">;
type Kind = "percent" | "naira" | "int" | "bool" | "clock";
interface Field {
  key: Key;
  label: string;
  kind: Kind;
  help?: string;
  unit?: string;
}

const SECTIONS: { title: string; blurb: string; fields: Field[] }[] = [
  {
    title: "Platform cut and fees",
    blurb: "What the market keeps, and the payment fees it passes on.",
    fields: [
      { key: "platform_cut_bps", label: "Platform cut", kind: "percent", help: "Of the item total, taken when an order is released. Orders already placed keep the rate they were placed at." },
      { key: "courier_price_tolerance_bps", label: "Courier price tolerance", kind: "percent", help: "A booking charged above the quote by more than this raises an alert." },
      { key: "fee_small_kobo", label: "Collection fee (small orders)", kind: "naira" },
      { key: "fee_large_kobo", label: "Collection fee (large orders)", kind: "naira" },
      { key: "fee_threshold_kobo", label: "Large-order threshold", kind: "naira" },
      { key: "stamp_duty_kobo", label: "Stamp duty", kind: "naira" },
      { key: "stamp_duty_threshold_kobo", label: "Stamp duty threshold", kind: "naira" },
    ],
  },
  {
    title: "Timing",
    blurb: "How long people have, and when things move on.",
    fields: [
      { key: "delivery_cutoff_minutes", label: "Next-day cut-off", kind: "clock", help: "Orders paid after this (Lagos time) are promised for the day after next." },
      { key: "ship_deadline_hours", label: "Seller ship deadline", kind: "int", unit: "hours" },
      { key: "release_after_hours", label: "Auto-release after delivery", kind: "int", unit: "hours" },
      { key: "backstop_days", label: "Backstop (unreleased orders flagged)", kind: "int", unit: "days" },
      { key: "payout_settlement_buffer_hours", label: "Payout settlement buffer", kind: "int", unit: "hours" },
      { key: "checkout_ttl_minutes", label: "Checkout expires after", kind: "int", unit: "minutes" },
      { key: "courier_quote_ttl_minutes", label: "Courier quote valid for", kind: "int", unit: "minutes" },
      { key: "jobs_ship_deadline_enabled", label: "Refund and strike when a seller misses the deadline", kind: "bool", help: "The kill switch for the ship-deadline job." },
    ],
  },
  {
    title: "Strikes and mutes",
    blurb: "When chat rule-breaking gets someone muted or sent to you.",
    fields: [
      { key: "strike_window_days", label: "Strikes count for", kind: "int", unit: "days" },
      { key: "mute_after_strikes", label: "Mute after", kind: "int", unit: "strikes" },
      { key: "mute_hours", label: "Mute lasts", kind: "int", unit: "hours" },
      { key: "owner_review_after_strikes", label: "Ask you to review a ban after", kind: "int", unit: "strikes", help: "Must be more than the mute threshold." },
    ],
  },
];

/** Stored value -> what the field shows. */
export function toDraft(kind: Kind, stored: number | boolean): string | boolean {
  if (kind === "bool") return stored as boolean;
  const n = stored as number;
  if (kind === "percent") return String(n / 100);
  if (kind === "naira") return String(n / 100);
  if (kind === "clock") return `${String(Math.floor(n / 60)).padStart(2, "0")}:${String(n % 60).padStart(2, "0")}`;
  return String(n);
}

/** What the field shows -> stored value; null when it is not a number. */
export function fromDraft(kind: Kind, draft: string | boolean): number | boolean | null {
  if (kind === "bool") return draft as boolean;
  const text = String(draft).trim();
  if (text === "") return null;
  if (kind === "clock") {
    const m = /^(\d{1,2}):(\d{2})$/.exec(text);
    return m ? Number(m[1]) * 60 + Number(m[2]) : null;
  }
  const n = Number(text);
  if (!Number.isFinite(n)) return null;
  if (kind === "percent" || kind === "naira") return Math.round(n * 100);
  return Number.isInteger(n) ? n : null;
}

const allFields = SECTIONS.flatMap((s) => s.fields);

export function Settings() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: ["settings"], queryFn: () => api.get<Settings>("/admin/settings") });
  const [draft, setDraft] = useState<Record<string, string | boolean>>({});
  const [stale, setStale] = useState(false);

  useEffect(() => {
    if (!q.data) return;
    setDraft(Object.fromEntries(allFields.map((f) => [f.key, toDraft(f.kind, q.data[f.key])])));
  }, [q.data]);

  const changes = useMemo(() => {
    if (!q.data) return {};
    const out: Record<string, number | boolean> = {};
    for (const f of allFields) {
      // Until the form has been filled from the server there is nothing to compare.
      if (draft[f.key] === undefined) continue;
      const v = fromDraft(f.kind, draft[f.key]);
      if (v !== null && v !== q.data[f.key]) out[f.key] = v;
    }
    return out;
  }, [draft, q.data]);
  const invalid = allFields
    .filter((f) => f.kind !== "bool" && draft[f.key] !== undefined && fromDraft(f.kind, draft[f.key]) === null)
    .map((f) => f.key);
  const count = Object.keys(changes).length;

  const save = useAction({
    run: () => api.patch<Settings>("/admin/settings", { ...changes, expected_updated_at: q.data?.updated_at }),
    success: "Settings saved. New orders use them from now on.",
    invalidate: [["settings"]],
  });
  const submit = () =>
    void save.run(undefined).catch((e) => {
      if (e instanceof ApiError && e.code === "stale_settings") setStale(true);
    });
  const reload = async () => {
    setStale(false);
    await qc.invalidateQueries({ queryKey: ["settings"] });
  };
  const reset = () => q.data && setDraft(Object.fromEntries(allFields.map((f) => [f.key, toDraft(f.kind, q.data[f.key])])));

  return (
    <>
      <PageHeader title="Settings" subtitle="Changes apply to new orders. Existing orders keep the terms they were placed with." />
      {q.isPending && <Card><SkeletonForm fields={8} /></Card>}
      {q.isError && <ErrorNote error={q.error} onRetry={() => void q.refetch()} />}
      {stale && (
        <Banner tone="warn" action={<button type="button" className="link" onClick={() => void reload()}>Reload</button>}>
          Someone else changed the settings after you opened them. Reload to see their changes; yours are not saved.
        </Banner>
      )}
      {q.data && (
        <div className="stack">
          <Banner tone="info">
            Payment methods on: {q.data.enabled_methods.join(", ") || "none"} · Payouts via {q.data.payout_provider}
          </Banner>
          {SECTIONS.map((s) => (
            <Card key={s.title} className="" >
              <h2>{s.title}</h2>
              <p className="muted">{s.blurb}</p>
              <div className="form-grid" style={{ marginTop: 6 }}>
                {s.fields.map((f) => (
                  <FieldInput key={f.key} field={f} value={draft[f.key] ?? ""} bad={invalid.includes(f.key)} changed={f.key in changes} onChange={(v) => setDraft((d) => ({ ...d, [f.key]: v }))} />
                ))}
              </div>
            </Card>
          ))}
          <PartnerCard />
        </div>
      )}
      <AnimatePresence>
        {count + invalid.length > 0 && (
          <motion.div className="savebar" initial={{ y: 80, opacity: 0 }} animate={{ y: 0, opacity: 1 }} exit={{ y: 80, opacity: 0 }} transition={spring.snappy} role="region" aria-label="Unsaved changes">
            <span>
              {count > 0 && (
                <>
                  <strong>{count}</strong> unsaved {count === 1 ? "change" : "changes"}
                </>
              )}
              {count > 0 && invalid.length > 0 && " · "}
              {invalid.length > 0 && "fix the highlighted fields first"}
            </span>
            <span className="row">
              <Button variant="ghost" onClick={reset} style={{ color: "inherit" }}>Discard</Button>
              <Button variant="primary" icon="check" loading={save.busy} done={save.done} disabled={invalid.length > 0} onClick={submit}>
                Save changes
              </Button>
            </span>
          </motion.div>
        )}
      </AnimatePresence>
    </>
  );
}

function FieldInput({ field, value, bad, changed, onChange }: { field: Field; value: string | boolean; bad: boolean; changed: boolean; onChange: (v: string | boolean) => void }) {
  const id = `f-${field.key}`;
  if (field.kind === "bool") {
    return (
      <div className="field" style={{ gridColumn: "1 / -1" }}>
        <span>{field.label}</span>
        <div className="row">
          <Toggle checked={value === true} onChange={onChange} label={field.label} />
          <span className="muted">{value === true ? "On" : "Off"}</span>
          {changed && <span className="badge badge-warn">changed</span>}
        </div>
        {field.help && <span className="hint">{field.help}</span>}
      </div>
    );
  }
  const text = String(value);
  return (
    <div className="field">
      <label htmlFor={id}>
        <span>{field.label}</span>
      </label>
      <motion.div animate={bad ? { x: [0, -5, 5, -3, 3, 0] } : { x: 0 }} transition={{ duration: 0.3 }}>
        <div style={{ position: "relative" }}>
          <input
            id={id}
            value={text}
            type={field.kind === "clock" ? "time" : "text"}
            inputMode={field.kind === "clock" ? undefined : "decimal"}
            onChange={(e) => onChange(e.target.value)}
            aria-invalid={bad || undefined}
            style={{ borderColor: bad ? "var(--bad)" : changed ? "var(--accent)" : undefined, paddingRight: field.kind === "clock" ? undefined : 64 }}
          />
          {field.kind !== "clock" && (
            <span className="muted" style={{ position: "absolute", right: 12, top: 9, pointerEvents: "none" }}>
              {field.kind === "percent" ? "%" : field.kind === "naira" ? "₦" : field.unit}
            </span>
          )}
        </div>
      </motion.div>
      {field.kind === "percent" && (
        <input className="range" type="range" min={0} max={30} step={0.5} value={Number(text) || 0} onChange={(e) => onChange(e.target.value)} aria-label={`${field.label} slider`} />
      )}
      {field.kind === "naira" && !bad && <span className="hint">{formatKobo(fromDraft("naira", text) as number)}</span>}
      {field.help && <span className="hint">{field.help}</span>}
    </div>
  );
}
