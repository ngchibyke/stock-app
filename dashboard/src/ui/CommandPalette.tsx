import { AnimatePresence, motion } from "motion/react";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { spring } from "../motion";
import { Icon, type IconName } from "./Icon";
import { useOverlay } from "./overlay";

export interface Command {
  id: string;
  label: string;
  group: string;
  icon: IconName;
  /** Extra words that should find this command ("dark" finds the theme switch). */
  keywords?: string;
  run: () => void;
}

/**
 * Jump to any screen, or run a quick action, from the keyboard: Ctrl or ⌘ + K
 * opens it. Type to narrow the list, arrows to move, Enter to go, Esc to close.
 */
export function CommandPalette({ open, onClose, commands }: { open: boolean; onClose: () => void; commands: Command[] }) {
  const panel = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  useOverlay(open, panel, onClose, "input");

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
    }
  }, [open]);

  const shown = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return commands.filter((c) => words.every((w) => `${c.label} ${c.group} ${c.keywords ?? ""}`.toLowerCase().includes(w)));
  }, [commands, query]);
  const current = Math.min(active, Math.max(0, shown.length - 1));

  useEffect(() => {
    panel.current?.querySelector<HTMLElement>(`[data-index="${current}"]`)?.scrollIntoView?.({ block: "nearest" });
  }, [current, shown]);

  const go = (c: Command | undefined) => {
    if (!c) return;
    onClose();
    c.run();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") (e.preventDefault(), setActive((current + 1) % Math.max(1, shown.length)));
    else if (e.key === "ArrowUp") (e.preventDefault(), setActive((current - 1 + shown.length) % Math.max(1, shown.length)));
    else if (e.key === "Enter") (e.preventDefault(), go(shown[current]));
  };

  let lastGroup = "";
  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div className="palette-backdrop" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.14 }} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
          <motion.div
            ref={panel}
            className="palette"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
            tabIndex={-1}
            initial={{ opacity: 0, y: -14, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -8, scale: 0.98, transition: { duration: 0.1 } }}
            transition={spring.snappy}
            onKeyDown={onKey}
          >
            <label className="palette-search">
              <Icon name="search" size={18} />
              <input
                ref={input}
                role="combobox"
                aria-expanded="true"
                aria-controls="palette-list"
                aria-activedescendant={shown[current] ? `palette-${shown[current].id}` : undefined}
                aria-label="Search screens and actions"
                placeholder="Go to a screen or run an action…"
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setActive(0);
                }}
              />
              <kbd className="kbd">Esc</kbd>
            </label>
            <div id="palette-list" role="listbox" aria-label="Results" className="palette-list">
              {shown.length === 0 && <div className="palette-empty">Nothing matches “{query}”.</div>}
              {shown.map((c, i) => {
                const header = c.group !== lastGroup;
                lastGroup = c.group;
                return (
                  <div key={c.id}>
                    {header && <div className="palette-group">{c.group}</div>}
                    <button
                      id={`palette-${c.id}`}
                      type="button"
                      role="option"
                      aria-selected={i === current}
                      tabIndex={-1}
                      data-index={i}
                      className={`palette-item ${i === current ? "active" : ""}`}
                      onMouseMove={() => setActive(i)}
                      onClick={() => go(c)}
                    >
                      <Icon name={c.icon} size={18} active={i === current} interactive={false} />
                      <span>{c.label}</span>
                      {i === current && (
                        <motion.span className="palette-enter" initial={{ opacity: 0, x: -4 }} animate={{ opacity: 1, x: 0 }}>
                          <Icon name="arrow" size={16} interactive={false} />
                        </motion.span>
                      )}
                    </button>
                  </div>
                );
              })}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
