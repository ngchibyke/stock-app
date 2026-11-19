import { AnimatePresence, motion } from "motion/react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { spring } from "../motion";
import { Button } from "./Button";
import { IconButton } from "./Button";

interface ModalProps {
  open: boolean;
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
}

/** A dialog: blurred backdrop, a spring-in panel, Esc and backdrop click close it. */
export function Modal({ open, title, onClose, children, footer }: ModalProps) {
  const titleId = useId();
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const previous = document.activeElement as HTMLElement | null;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
      if (e.key === "Tab" && panel.current) {
        const items = panel.current.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        );
        if (items.length === 0) return;
        const first = items[0];
        const last = items[items.length - 1];
        if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
    };
    document.addEventListener("keydown", onKey);
    // Focus the first field, else the panel.
    const t = setTimeout(() => {
      // Never steal focus from a field the person has already started typing in.
      if (panel.current?.contains(document.activeElement)) return;
      const field = panel.current?.querySelector<HTMLElement>("input, textarea, select");
      (field ?? panel.current)?.focus();
    }, 30);
    return () => {
      clearTimeout(t);
      document.removeEventListener("keydown", onKey);
      previous?.focus?.();
    };
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="modal-backdrop"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          onMouseDown={(e) => e.target === e.currentTarget && onClose()}
        >
          <motion.div
            ref={panel}
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby={titleId}
            tabIndex={-1}
            initial={{ opacity: 0, y: 24, scale: 0.94 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.97, transition: { duration: 0.15 } }}
            transition={spring.snappy}
          >
            <header className="modal-head">
              <h2 id={titleId}>{title}</h2>
              <IconButton icon="x" label="Close" onClick={onClose} />
            </header>
            <div className="modal-body">{children}</div>
            {footer && <footer className="modal-foot">{footer}</footer>}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

interface ConfirmProps {
  open: boolean;
  title: string;
  /** What will happen, in plain words. Money actions put the exact amount here. */
  summary: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  /** The server needs a note for every owner action. Hidden when false. */
  noteLabel?: string | false;
  onClose: () => void;
  /** Runs on confirm with the note; the dialog closes when it resolves. */
  onConfirm: (note: string) => Promise<void>;
  /** Extra fields shown above the note. */
  children?: ReactNode;
}

/** A confirmation that asks for a note, shows a busy button, and shakes if refused. */
export function ConfirmDialog({
  open,
  title,
  summary,
  confirmLabel,
  danger,
  noteLabel = "Note (kept in the audit log)",
  onClose,
  onConfirm,
  children,
}: ConfirmProps) {
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [shake, setShake] = useState(0);
  const needNote = noteLabel !== false;
  const ready = !needNote || note.trim().length > 0;

  useEffect(() => {
    if (open) setNote("");
  }, [open]);

  const submit = async () => {
    if (!ready) {
      setShake((s) => s + 1);
      return;
    }
    setBusy(true);
    try {
      await onConfirm(note.trim());
      onClose();
    } catch {
      // The caller shows the error; the dialog stays so nothing is lost.
      setShake((s) => s + 1);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      open={open}
      title={title}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button variant={danger ? "danger" : "primary"} loading={busy} onClick={submit}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      <motion.div
        key={shake}
        animate={shake ? { x: [0, -8, 8, -5, 5, 0] } : undefined}
        transition={{ duration: 0.4 }}
      >
        <div className="summary">{summary}</div>
        {children}
        {needNote && (
          <label className="field">
            <span>{noteLabel}</span>
            <textarea
              value={note}
              onChange={(e) => setNote(e.target.value)}
              rows={3}
              maxLength={500}
              placeholder="Why are you doing this?"
            />
            <small>{note.length}/500</small>
          </label>
        )}
      </motion.div>
    </Modal>
  );
}
