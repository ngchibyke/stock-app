// Small interactive pieces: cards, badges, tabs, counters, skeletons, empty
// states, a segmented control and a copy chip.
import { animate, motion, useInView, useMotionValue, useTransform } from "motion/react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { formatKobo } from "../api";
import { itemVariants, listVariants, spring } from "../motion";
import { Icon, type IconName } from "./Icon";

export function Card({ children, className, hover = false }: { children: ReactNode; className?: string; hover?: boolean }) {
  return (
    <motion.section
      className={`card ${className ?? ""}`}
      whileHover={hover ? { y: -3 } : undefined}
      transition={spring.snappy}
    >
      {children}
    </motion.section>
  );
}

type Tone = "neutral" | "good" | "warn" | "bad" | "info";

export function Badge({ tone = "neutral", children }: { tone?: Tone; children: ReactNode }) {
  return (
    <motion.span
      className={`badge badge-${tone}`}
      initial={{ scale: 0.8, opacity: 0 }}
      animate={{ scale: 1, opacity: 1 }}
      transition={spring.bouncy}
    >
      {children}
    </motion.span>
  );
}

/** Maps a status word from the API to a badge colour. */
export function toneFor(status: string): Tone {
  if (/(completed|active|matched|verified|booked|delivered|successful|decided|resolved|acked)/.test(status)) return "good";
  if (/(failed|suspended|banned|mismatch|refunded|cancelled|reversed|upheld|locked)/.test(status)) return "bad";
  if (/(pending|awaiting|held|manual|unknown|unavailable|attention|open)/.test(status)) return "warn";
  return "neutral";
}

export function StatusBadge({ status }: { status: string }) {
  return <Badge tone={toneFor(status)}>{status.replace(/_/g, " ")}</Badge>;
}

/** A number that rolls up to its value when it scrolls into view. */
export function Counter({ to, format = (n: number) => Math.round(n).toLocaleString("en-NG") }: { to: number; format?: (n: number) => string }) {
  const ref = useRef<HTMLSpanElement>(null);
  const seen = useInView(ref, { once: true });
  const value = useMotionValue(0);
  const text = useTransform(value, format);
  useEffect(() => {
    if (!seen) return;
    const controls = animate(value, to, { duration: 0.9, ease: [0.22, 1, 0.36, 1] });
    return () => controls.stop();
  }, [seen, to, value]);
  return (
    <motion.span ref={ref} aria-label={format(to)}>
      {text}
    </motion.span>
  );
}

export function Money({ kobo, signed = false }: { kobo: number | null | undefined; signed?: boolean }) {
  const tone = signed && kobo ? (kobo < 0 ? "neg" : "pos") : "";
  return <span className={`money ${tone}`}>{formatKobo(kobo)}</span>;
}

export { Skeleton } from "./Skeleton";

export function EmptyState({ icon, title, hint, action }: { icon: IconName; title: string; hint?: string; action?: ReactNode }) {
  return (
    <motion.div className="empty" initial={{ opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1 }} transition={spring.gentle}>
      <motion.div className="empty-icon" animate={{ y: [0, -5, 0] }} transition={{ repeat: Infinity, duration: 3.2, ease: "easeInOut" }}>
        <Icon name={icon} size={30} />
      </motion.div>
      <strong>{title}</strong>
      {hint && <span>{hint}</span>}
      {action && <div style={{ marginTop: 10 }}>{action}</div>}
    </motion.div>
  );
}

export function ErrorNote({ error, onRetry }: { error: unknown; onRetry?: () => void }) {
  const message = error instanceof Error ? error.message : "Something went wrong.";
  return (
    <motion.div className="error-note" role="alert" initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={spring.snappy}>
      <Icon name="warn" size={18} active />
      <span>{message}</span>
      {onRetry && (
        <button type="button" className="link" onClick={onRetry}>
          Try again
        </button>
      )}
    </motion.div>
  );
}

/** Tabs with one underline that slides to the selected tab. */
export function Tabs<T extends string>({
  value,
  onChange,
  tabs,
  id,
}: {
  value: T;
  onChange: (v: T) => void;
  tabs: { value: T; label: string; count?: number }[];
  id: string;
}) {
  return (
    <div className="tabs" role="tablist">
      {tabs.map((t) => (
        <button
          key={t.value}
          type="button"
          role="tab"
          aria-selected={t.value === value}
          className={`tab ${t.value === value ? "on" : ""}`}
          onClick={() => onChange(t.value)}
        >
          {t.label}
          {t.count !== undefined && t.count > 0 && <span className="tab-count">{t.count}</span>}
          {t.value === value && <motion.span layoutId={`tab-${id}`} className="tab-line" transition={spring.snappy} />}
        </button>
      ))}
    </div>
  );
}

/** A staggered list container; wrap each child in <Item>. */
export function Stagger({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <motion.div className={className} variants={listVariants} initial="hidden" animate="show">
      {children}
    </motion.div>
  );
}
export function Item({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <motion.div className={className} variants={itemVariants}>
      {children}
    </motion.div>
  );
}

/** A value that copies itself on click and says so. */
export function CopyChip({ value, label }: { value: string; label?: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <motion.button
      type="button"
      className="chip"
      whileHover={{ y: -1 }}
      whileTap={{ scale: 0.95 }}
      onClick={async () => {
        try {
          await navigator.clipboard.writeText(value);
        } catch {
          // Clipboard blocked: the value is still on screen to select.
        }
        setCopied(true);
        setTimeout(() => setCopied(false), 1400);
      }}
      title="Copy"
    >
      <Icon name={copied ? "check" : "copy"} size={14} active={copied} interactive={false} />
      <span className="mono">{label ?? value}</span>
    </motion.button>
  );
}

/** A switch whose knob springs across. */
export function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} aria-label={label} className={`toggle ${checked ? "on" : ""}`} onClick={() => onChange(!checked)}>
      <motion.span className="toggle-knob" layout transition={spring.bouncy} />
    </button>
  );
}

/** A pill-shaped filter: the highlight slides to the chosen option. */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  id,
  label,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  id: string;
  label: string;
}) {
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map((o) => (
        <button key={o.value} type="button" className={o.value === value ? "on" : ""} aria-pressed={o.value === value} onClick={() => onChange(o.value)}>
          {o.value === value && <motion.span layoutId={`seg-${id}`} className="seg-pill" transition={spring.snappy} />}
          {o.label}
        </button>
      ))}
    </div>
  );
}
