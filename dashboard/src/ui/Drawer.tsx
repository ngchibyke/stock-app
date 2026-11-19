import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useEffect } from "react";
import { spring } from "../motion";
import { IconButton } from "./Button";

/** A panel that springs in from the right. Esc and a click outside close it. */
export function Drawer({ open, title, onClose, children, wide = false }: { open: boolean; title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, onClose]);
  return (
    <AnimatePresence>
      {open && (
        <motion.div className="drawer-backdrop" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.16 }} onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
          <motion.aside
            className={`drawer ${wide ? "wide" : ""}`}
            role="dialog"
            aria-label={title}
            initial={{ x: "100%" }}
            animate={{ x: 0 }}
            exit={{ x: "100%", transition: { duration: 0.18 } }}
            transition={spring.snappy}
            drag="x"
            dragConstraints={{ left: 0, right: 0 }}
            dragElastic={{ left: 0, right: 0.6 }}
            onDragEnd={(_, info) => info.offset.x > 120 && onClose()}
          >
            <header className="drawer-head">
              <h2>{title}</h2>
              <IconButton icon="x" label="Close" onClick={onClose} />
            </header>
            <div className="drawer-body">{children}</div>
          </motion.aside>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
