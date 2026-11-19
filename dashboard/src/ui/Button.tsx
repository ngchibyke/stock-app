import { AnimatePresence, motion } from "motion/react";
import { type ComponentProps, type ReactNode } from "react";
import { spring } from "../motion";
import { Icon, type IconName } from "./Icon";

type Variant = "primary" | "secondary" | "ghost" | "danger";

interface ButtonProps extends Omit<ComponentProps<typeof motion.button>, "children" | "ref"> {
  variant?: Variant;
  icon?: IconName;
  /** Replaces the icon with a spinner and blocks clicks. */
  loading?: boolean;
  /** Shows a check for a moment (set by the caller after success). */
  done?: boolean;
  children?: ReactNode;
}

export function Button({
  variant = "secondary",
  icon,
  loading = false,
  done = false,
  disabled,
  children,
  className,
  type = "button",
  ...rest
}: ButtonProps) {
  const blocked = disabled || loading;
  const lead = loading ? "spinner" : done ? "done" : icon ? "icon" : null;
  return (
    <motion.button
      type={type}
      className={`btn btn-${variant} ${className ?? ""}`}
      disabled={blocked}
      aria-busy={loading || undefined}
      initial="rest"
      whileHover={blocked ? undefined : "hover"}
      whileTap={blocked ? undefined : "tap"}
      animate="rest"
      variants={{ rest: { y: 0, scale: 1 }, hover: { y: -1.5 }, tap: { y: 0, scale: 0.97 } }}
      transition={spring.snappy}
      {...rest}
    >
      <AnimatePresence initial={false} mode="popLayout">
        {lead && (
          <motion.span
            key={lead}
            className="btn-lead"
            initial={{ opacity: 0, scale: 0.4, rotate: -45 }}
            animate={{ opacity: 1, scale: 1, rotate: 0 }}
            exit={{ opacity: 0, scale: 0.4, rotate: 45 }}
            transition={spring.snappy}
          >
            {lead === "spinner" ? (
              <span className="spinner" role="status" aria-label="Working" />
            ) : lead === "done" ? (
              <Icon name="check" size={16} interactive={false} />
            ) : (
              <Icon name={icon as IconName} size={16} interactive={false} />
            )}
          </motion.span>
        )}
      </AnimatePresence>
      {children && <span className="btn-label">{children}</span>}
    </motion.button>
  );
}

/** A round icon-only button with a tooltip and a ripple of its own. */
export function IconButton({
  icon,
  label,
  onClick,
  variant = "ghost",
  disabled,
}: {
  icon: IconName;
  label: string;
  onClick?: () => void;
  variant?: Variant;
  disabled?: boolean;
}) {
  return (
    <motion.button
      type="button"
      className={`icon-btn btn-${variant}`}
      aria-label={label}
      data-tip={label}
      onClick={onClick}
      disabled={disabled}
      initial="rest"
      whileHover="hover"
      whileTap="tap"
      variants={{ rest: { scale: 1 }, hover: { scale: 1.1 }, tap: { scale: 0.9 } }}
      transition={spring.bouncy}
    >
      <Icon name={icon} size={18} interactive={false} />
    </motion.button>
  );
}
