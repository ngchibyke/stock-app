// Shared plumbing for everything that floats above the page: dialogs, drawers,
// menus, tooltips and the command palette.
import { type RefObject, useEffect, useLayoutEffect, useRef, useState } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * While `open`: Esc calls `onClose`, Tab stays inside `panel`, the page behind
 * does not scroll, and focus goes back to where it was when the overlay closes.
 * Focus lands on the first field (or `initial`, or the panel itself) unless the
 * person has already put it somewhere inside.
 */
export function useOverlay(open: boolean, panel: RefObject<HTMLElement | null>, onClose: () => void, initial?: string) {
  // The latest `onClose`, without re-running the effect (which would move focus) each time a caller re-renders.
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
        return;
      }
      if (e.key !== "Tab" || !panel.current) return;
      const items = [...panel.current.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null || el === document.activeElement);
      if (items.length === 0) {
        e.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && (document.activeElement === first || document.activeElement === panel.current)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    const t = setTimeout(() => {
      // Never steal focus from a field the person has already started typing in.
      if (panel.current?.contains(document.activeElement)) return;
      const target = (initial ? panel.current?.querySelector<HTMLElement>(initial) : null) ?? panel.current?.querySelector<HTMLElement>("input, textarea, select");
      (target ?? panel.current)?.focus();
    }, 30);
    return () => {
      clearTimeout(t);
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = overflow;
      previous?.focus?.();
    };
  }, [open, panel, initial]);
}

/** Calls `onOutside` for a press anywhere outside every one of `refs`. */
export function useOutsidePress(open: boolean, refs: RefObject<HTMLElement | null>[], onOutside: () => void) {
  useEffect(() => {
    if (!open) return;
    const on = (e: MouseEvent | TouchEvent) => {
      const target = e.target as Node;
      if (refs.every((r) => !r.current?.contains(target))) onOutside();
    };
    document.addEventListener("mousedown", on);
    document.addEventListener("touchstart", on);
    return () => {
      document.removeEventListener("mousedown", on);
      document.removeEventListener("touchstart", on);
    };
  }, [open, refs, onOutside]);
}

export interface Placement {
  top: number;
  left: number;
  /** Where the popover ended up, for its arrow and its entrance direction. */
  side: "top" | "bottom";
}

/**
 * Where to put a popover of `size` against `anchor`: below it (or above, when
 * there is no room), kept inside the window with an 8px margin. `align` picks
 * which edge of the anchor the popover lines up with.
 */
export function place(anchor: DOMRect, size: { width: number; height: number }, align: "start" | "end" | "center" = "start", prefer: "top" | "bottom" = "bottom", gap = 8): Placement {
  const margin = 8;
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const room = { below: vh - anchor.bottom - margin, above: anchor.top - margin };
  const side = prefer === "bottom" ? (room.below >= size.height + gap || room.below >= room.above ? "bottom" : "top") : room.above >= size.height + gap || room.above >= room.below ? "top" : "bottom";
  const top = side === "bottom" ? anchor.bottom + gap : anchor.top - size.height - gap;
  let left = align === "start" ? anchor.left : align === "end" ? anchor.right - size.width : anchor.left + anchor.width / 2 - size.width / 2;
  left = Math.max(margin, Math.min(left, vw - size.width - margin));
  return { top: Math.max(margin, Math.min(top, vh - size.height - margin)), left, side };
}

/** Measures a floating element once it is on screen and again if its content changes. */
export function useMeasured(ref: RefObject<HTMLElement | null>, active: boolean): { width: number; height: number } {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    if (!active || !ref.current) return;
    const el = ref.current;
    const measure = () => setSize((s) => (s.width === el.offsetWidth && s.height === el.offsetHeight ? s : { width: el.offsetWidth, height: el.offsetHeight }));
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref, active]);
  return size;
}
