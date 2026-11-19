// Toasts: spring in from the right, swipe to dismiss, a progress line that
// shrinks until they go, and hovering pauses the timer.
import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, createContext, useCallback, useContext, useMemo, useRef, useState } from "react";
import { ApiError } from "../api";
import { spring } from "../motion";
import { Icon, type IconName } from "./Icon";

type Kind = "success" | "error" | "info";
interface Toast {
  id: number;
  kind: Kind;
  text: string;
}

interface ToastApi {
  success: (text: string) => void;
  info: (text: string) => void;
  /** Shows an ApiError's message, or a fallback for anything else. */
  error: (e: unknown) => void;
}

const ToastContext = createContext<ToastApi | null>(null);

export function useToasts(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToasts must be used inside ToastProvider");
  return ctx;
}

const LIFETIME = 5000;
const icons: Record<Kind, IconName> = { success: "check", error: "warn", info: "info" };

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const next = useRef(1);
  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);
  const push = useCallback((kind: Kind, text: string) => {
    const id = next.current++;
    setToasts((t) => [...t.slice(-3), { id, kind, text }]);
  }, []);

  const api = useMemo<ToastApi>(
    () => ({
      success: (t) => push("success", t),
      info: (t) => push("info", t),
      error: (e) => push("error", e instanceof ApiError ? e.message : "Something went wrong. Try again."),
    }),
    [push],
  );

  return (
    <ToastContext.Provider value={api}>
      {children}
      <div className="toasts" aria-live="polite">
        <AnimatePresence initial={false}>
          {toasts.map((t) => (
            <ToastItem key={t.id} toast={t} onDone={() => dismiss(t.id)} />
          ))}
        </AnimatePresence>
      </div>
    </ToastContext.Provider>
  );
}

function ToastItem({ toast, onDone }: { toast: Toast; onDone: () => void }) {
  const [paused, setPaused] = useState(false);
  return (
    <motion.div
      layout
      className={`toast toast-${toast.kind}`}
      role={toast.kind === "error" ? "alert" : "status"}
      initial={{ opacity: 0, x: 60, scale: 0.9 }}
      animate={{ opacity: 1, x: 0, scale: 1 }}
      exit={{ opacity: 0, x: 80, scale: 0.9, transition: { duration: 0.18 } }}
      transition={spring.snappy}
      drag="x"
      dragConstraints={{ left: 0, right: 0 }}
      dragElastic={{ left: 0, right: 0.8 }}
      onDragEnd={(_, info) => info.offset.x > 80 && onDone()}
      onHoverStart={() => setPaused(true)}
      onHoverEnd={() => setPaused(false)}
    >
      <Icon name={icons[toast.kind]} size={18} active />
      <span className="toast-text">{toast.text}</span>
      <motion.span
        className="toast-timer"
        initial={{ scaleX: 1 }}
        animate={{ scaleX: paused ? undefined : 0 }}
        transition={{ duration: LIFETIME / 1000, ease: "linear" }}
        onAnimationComplete={onDone}
      />
    </motion.div>
  );
}
