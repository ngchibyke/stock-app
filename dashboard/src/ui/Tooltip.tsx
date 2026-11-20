import { AnimatePresence, motion } from "motion/react";
import { type ReactElement, type ReactNode, cloneElement, useCallback, useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { place, useMeasured } from "./overlay";

/** True when focus arrived the way a keyboard user would cause it (jsdom cannot tell, so it says yes). */
function keyboardFocus(el: EventTarget): boolean {
  try {
    return (el as Element).matches(":focus-visible");
  } catch {
    return true;
  }
}

/**
 * A small label for an icon or a clipped value. It opens after a short pause on
 * hover (so moving across a toolbar does not flash every tip) and at once on
 * keyboard focus, closes on Esc, and is drawn in a portal so a table's or a
 * drawer's overflow never clips it. The child keeps its own events.
 */
export function Tooltip({ label, children, delay = 350, side = "top" }: { label: ReactNode; children: ReactElement<Record<string, unknown>>; delay?: number; side?: "top" | "bottom" }) {
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const tip = useRef<HTMLDivElement>(null);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [open, setOpen] = useState(false);
  const [rect, setRect] = useState<DOMRect | null>(null);
  const size = useMeasured(tip, open);

  const show = useCallback(
    (wait: number) => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        if (anchor.current) setRect(anchor.current.getBoundingClientRect());
        setOpen(true);
      }, wait);
    },
    [],
  );
  const hide = useCallback(() => {
    clearTimeout(timer.current);
    setOpen(false);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && hide();
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", hide, true);
    return () => {
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", hide, true);
    };
  }, [open, hide]);
  useEffect(() => () => clearTimeout(timer.current), []);

  const at = rect && size.width ? place(rect, size, "center", side, 8) : null;
  return (
    <>
      <span ref={anchor} className="tip-anchor" onMouseEnter={() => show(delay)} onMouseLeave={hide} onFocus={(e) => keyboardFocus(e.target) && show(0)} onBlur={hide} onMouseDown={hide}>
        {cloneElement(children, { "aria-describedby": open ? id : undefined })}
      </span>
      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={tip}
              id={id}
              role="tooltip"
              className="tooltip"
              style={{ top: at?.top ?? -9999, left: at?.left ?? -9999 }}
              initial={{ opacity: 0, y: side === "top" ? 4 : -4, scale: 0.96 }}
              animate={{ opacity: at ? 1 : 0, y: 0, scale: 1 }}
              exit={{ opacity: 0, transition: { duration: 0.08 } }}
              transition={{ duration: 0.14 }}
            >
              {label}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
