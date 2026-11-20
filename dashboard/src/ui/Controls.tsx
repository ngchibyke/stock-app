// Small form and display pieces that match the rest of the kit: a checkbox whose
// tick draws itself, a radio card, an avatar, a keyboard hint, an inline banner.
import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useId } from "react";
import { spring } from "../motion";
import { Icon, type IconName } from "./Icon";

/** A checkbox: the box fills with a spring and the tick draws itself in. */
export function Checkbox({ checked, onChange, label, hint, disabled }: { checked: boolean; onChange: (v: boolean) => void; label: ReactNode; hint?: ReactNode; disabled?: boolean }) {
  const id = useId();
  return (
    <label className={`check ${disabled ? "disabled" : ""}`} htmlFor={id}>
      <input id={id} type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <motion.span className="check-box" animate={{ scale: checked ? [1, 1.14, 1] : 1 }} transition={{ duration: 0.25 }} aria-hidden="true">
        <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
          <motion.path d="m5 12.5 4.5 4.5L19 7.5" initial={false} animate={{ pathLength: checked ? 1 : 0, opacity: checked ? 1 : 0 }} transition={{ duration: 0.22 }} />
        </svg>
      </motion.span>
      <span className="check-text">
        {label}
        {hint && <small>{hint}</small>}
      </span>
    </label>
  );
}

/** One option of a radio group, drawn as a card. Put several in a `role="radiogroup"`. */
export function RadioCard({ name, value, checked, onChange, title, children }: { name: string; value: string; checked: boolean; onChange: (v: string) => void; title: ReactNode; children?: ReactNode }) {
  return (
    <label className={`choice ${checked ? "on" : ""}`}>
      <input type="radio" name={name} value={value} checked={checked} onChange={() => onChange(value)} />
      <span>
        <strong>{title}</strong>
        {children && <span className="hint" style={{ display: "block" }}>{children}</span>}
      </span>
    </label>
  );
}

const HUES = [248, 262, 200, 160, 24, 340, 190, 290];

/** A round initial on a colour chosen from the name, so a person is recognisable at a glance. */
export function Avatar({ name, size = 30 }: { name: string; size?: number }) {
  const hue = HUES[[...name].reduce((n, c) => n + c.charCodeAt(0), 0) % HUES.length];
  return (
    <span className="avatar" aria-hidden="true" style={{ width: size, height: size, fontSize: size * 0.42, background: `hsl(${hue} 80% 92%)`, color: `hsl(${hue} 55% 38%)` }}>
      {name.trim().slice(0, 1).toUpperCase() || "?"}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="kbd">{children}</kbd>;
}

type BannerTone = "info" | "good" | "warn" | "bad";
const bannerIcon: Record<BannerTone, IconName> = { info: "info", good: "check", warn: "warn", bad: "warn" };

/** An inline message about the page or a form: what happened, and what to do about it. */
export function Banner({ tone = "info", title, children, action, onDismiss }: { tone?: BannerTone; title?: ReactNode; children?: ReactNode; action?: ReactNode; onDismiss?: () => void }) {
  return (
    <AnimatePresence initial={false}>
      <motion.div
        className={`banner banner-${tone}`}
        role={tone === "bad" || tone === "warn" ? "alert" : "status"}
        initial={{ opacity: 0, y: -8, height: 0 }}
        animate={{ opacity: 1, y: 0, height: "auto" }}
        exit={{ opacity: 0, height: 0 }}
        transition={spring.snappy}
      >
        <Icon name={bannerIcon[tone]} size={18} active />
        <div className="banner-body">
          {title && <strong>{title}</strong>}
          {children && <span>{children}</span>}
        </div>
        {action}
        {onDismiss && (
          <button type="button" className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
            <Icon name="x" size={16} interactive={false} />
          </button>
        )}
      </motion.div>
    </AnimatePresence>
  );
}
