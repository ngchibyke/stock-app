// Motion tokens. Personality: crisp and efficient (an admin tool): short
// durations, snappy springs, small travel. Everything animated reads these, so
// the feel changes in one place.
import type { Transition, Variants } from "motion/react";

export const duration = { instant: 0.1, fast: 0.2, base: 0.3, slow: 0.5 } as const;
export const ease = {
  out: [0.22, 1, 0.36, 1],
  in: [0.64, 0, 0.78, 0],
  inOut: [0.65, 0, 0.35, 1],
} as const;

export const spring = {
  snappy: { type: "spring", stiffness: 420, damping: 30 },
  gentle: { type: "spring", stiffness: 160, damping: 22 },
  bouncy: { type: "spring", stiffness: 320, damping: 16 },
} as const satisfies Record<string, Transition>;

export const stagger = { tight: 0.03, base: 0.05 } as const;

/** A list that fades its children in one after another. */
export const listVariants: Variants = {
  hidden: {},
  show: { transition: { staggerChildren: stagger.tight, delayChildren: 0.04 } },
};
export const itemVariants: Variants = {
  hidden: { opacity: 0, y: 8 },
  show: { opacity: 1, y: 0, transition: { duration: duration.base, ease: ease.out } },
};

/** A route change: a small rise and fade. */
export const pageVariants: Variants = {
  initial: { opacity: 0, y: 10 },
  enter: { opacity: 1, y: 0, transition: { duration: duration.base, ease: ease.out } },
  exit: { opacity: 0, y: -6, transition: { duration: duration.fast, ease: ease.in } },
};
