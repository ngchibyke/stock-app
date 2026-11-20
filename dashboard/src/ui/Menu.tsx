import { AnimatePresence, motion } from "motion/react";
import { type KeyboardEvent, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { spring } from "../motion";
import { IconButton } from "./Button";
import { Icon, type IconName } from "./Icon";
import { place, useMeasured, useOutsidePress } from "./overlay";

export type MenuItem =
  | { label: string; icon?: IconName; onSelect: () => void; danger?: boolean; disabled?: boolean; hint?: string }
  | "separator";

/**
 * A button that opens a list of actions. Arrow keys move, Home and End jump,
 * typing a letter jumps to the next item that starts with it, Enter or Space
 * picks, Esc closes and returns focus to the button. Drawn in a portal, so a
 * table row never clips it, and it flips above the button when there is no room
 * below.
 */
export function Menu({ label, items, icon = "more", align = "end" }: { label: string; items: MenuItem[]; icon?: IconName; align?: "start" | "end" }) {
  const menuId = useId();
  const wrap = useRef<HTMLSpanElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const size = useMeasured(list, open);

  const actionable = useMemo(() => items.map((it, i) => (it !== "separator" && !it.disabled ? i : -1)).filter((i) => i >= 0), [items]);
  const close = useCallback((refocus = true) => {
    setOpen(false);
    if (refocus) wrap.current?.querySelector<HTMLElement>("button")?.focus();
  }, []);
  const outside = useCallback(() => close(false), [close]);
  useOutsidePress(open, [wrap, list], outside);

  const openMenu = (first: "first" | "last" = "first") => {
    if (wrap.current) setRect(wrap.current.getBoundingClientRect());
    setActive(first === "first" ? (actionable[0] ?? 0) : (actionable[actionable.length - 1] ?? 0));
    setOpen(true);
  };

  useEffect(() => {
    if (!open) return;
    const t = setTimeout(() => list.current?.focus(), 0);
    const hide = () => setOpen(false);
    window.addEventListener("resize", hide);
    window.addEventListener("scroll", hide, true);
    return () => {
      clearTimeout(t);
      window.removeEventListener("resize", hide);
      window.removeEventListener("scroll", hide, true);
    };
  }, [open]);

  const step = (dir: 1 | -1) => {
    const at = actionable.indexOf(active);
    setActive(actionable[(at + dir + actionable.length) % actionable.length]);
  };
  const choose = (i: number) => {
    const it = items[i];
    if (it === "separator" || it.disabled) return;
    close();
    it.onSelect();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") (e.preventDefault(), step(1));
    else if (e.key === "ArrowUp") (e.preventDefault(), step(-1));
    else if (e.key === "Home") (e.preventDefault(), setActive(actionable[0]));
    else if (e.key === "End") (e.preventDefault(), setActive(actionable[actionable.length - 1]));
    else if (e.key === "Enter" || e.key === " ") (e.preventDefault(), choose(active));
    else if (e.key === "Escape") (e.preventDefault(), e.stopPropagation(), close());
    else if (e.key === "Tab") close(false);
    else if (e.key.length === 1 && /\S/.test(e.key)) {
      const from = actionable.indexOf(active);
      const order = [...actionable.slice(from + 1), ...actionable.slice(0, from + 1)];
      const hit = order.find((i) => (items[i] as { label: string }).label.toLowerCase().startsWith(e.key.toLowerCase()));
      if (hit !== undefined) setActive(hit);
    }
  };

  const at = rect && size.width ? place(rect, size, align) : null;
  return (
    <>
      <span ref={wrap} className="menu-anchor" onKeyDown={(e) => (e.key === "ArrowDown" || e.key === "ArrowUp") && !open && (e.preventDefault(), openMenu(e.key === "ArrowDown" ? "first" : "last"))}>
        <IconButton icon={icon} label={label} onClick={() => (open ? close() : openMenu())} />
      </span>
      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={list}
              id={menuId}
              role="menu"
              aria-label={label}
              tabIndex={-1}
              className="menu"
              style={{ top: at?.top ?? -9999, left: at?.left ?? -9999, transformOrigin: at?.side === "top" ? "bottom" : "top" }}
              initial={{ opacity: 0, scale: 0.94, y: at?.side === "top" ? 6 : -6 }}
              animate={{ opacity: at ? 1 : 0, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96, transition: { duration: 0.1 } }}
              transition={spring.snappy}
              onKeyDown={onKey}
              aria-activedescendant={`${menuId}-${active}`}
            >
              {items.map((it, i) =>
                it === "separator" ? (
                  <div key={`s${i}`} role="separator" className="menu-sep" />
                ) : (
                  <button
                    key={it.label}
                    id={`${menuId}-${i}`}
                    type="button"
                    role="menuitem"
                    tabIndex={-1}
                    disabled={it.disabled}
                    className={`menu-item ${it.danger ? "danger" : ""} ${i === active ? "active" : ""}`}
                    onMouseEnter={() => !it.disabled && setActive(i)}
                    onClick={() => choose(i)}
                  >
                    {it.icon && <Icon name={it.icon} size={16} interactive={false} />}
                    <span>{it.label}</span>
                    {it.hint && <kbd className="kbd">{it.hint}</kbd>}
                  </button>
                ),
              )}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
