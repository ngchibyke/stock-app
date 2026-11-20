// Interactive icons. Each icon is a set of SVG paths plus a behaviour that plays
// on hover, on press, and once when it becomes `active`. Because the icon's
// inner elements use the variant names `hover` and `tap`, a parent motion
// element (a button, a nav link) that animates to those variants drives the icon
// too: hovering the whole button animates its icon, not just the glyph.
import { type Variants, motion } from "motion/react";
import { type ReactNode } from "react";

export type Behaviour = "spin" | "wiggle" | "bounce" | "pulse" | "draw" | "slide" | "turn" | "tilt" | "pop";

const keyframes = (b: Behaviour): Variants => {
  const t = { duration: 0.55, ease: [0.22, 1, 0.36, 1] as const };
  switch (b) {
    case "spin":
      return { rest: { rotate: 0 }, hover: { rotate: 90, transition: { type: "spring", stiffness: 160, damping: 12 } }, tap: { rotate: 180 } };
    case "wiggle":
      return { rest: { rotate: 0 }, hover: { rotate: [0, -16, 13, -9, 4, 0], transition: { duration: 0.7 } }, tap: { scale: 0.88 } };
    case "bounce":
      return { rest: { y: 0 }, hover: { y: [0, -3.5, 0, -1.5, 0], transition: { duration: 0.6 } }, tap: { y: 1.5, scale: 0.92 } };
    case "pulse":
      return { rest: { scale: 1 }, hover: { scale: [1, 1.22, 1, 1.12, 1], transition: { duration: 0.6 } }, tap: { scale: 0.85 } };
    case "draw":
      return { rest: { pathLength: 1, opacity: 1 }, hover: { pathLength: [0, 1], opacity: [0.4, 1], transition: t }, tap: { scale: 0.9 } };
    case "slide":
      return { rest: { x: 0 }, hover: { x: [0, 3.5, 0], transition: { duration: 0.55 } }, tap: { x: 4 } };
    case "turn":
      return { rest: { rotate: 0 }, hover: { rotate: 180, transition: { type: "spring", stiffness: 140, damping: 14 } }, tap: { rotate: 270 } };
    case "tilt":
      return { rest: { rotate: 0, y: 0 }, hover: { rotate: [0, -12, 9, -4, 0], y: [0, -1, 0], transition: { duration: 0.65 } }, tap: { scale: 0.9 } };
    case "pop":
      return { rest: { scale: 1 }, hover: { scale: [1, 1.3, 0.95, 1], transition: { duration: 0.45 } }, tap: { scale: 0.8 } };
  }
};

interface Glyph {
  behaviour: Behaviour;
  /** Paths drawn with the stroke. */
  paths: string[];
  /** Extra shapes (circles, dots) drawn by hand. */
  extra?: ReactNode;
}

// 24x24, 1.8 stroke, round caps: one family.
export const glyphs = {
  home: { behaviour: "bounce", paths: ["M3 11 12 3l9 8", "M5 10v10h5v-6h4v6h5V10"] },
  key: { behaviour: "tilt", paths: ["M14 10a4 4 0 1 0-3.5 4L21 14v3h-3v3h-3v-3l-2.5-2.5"], extra: <circle cx="7.5" cy="9" r="0.8" fill="currentColor" stroke="none" /> },
  users: { behaviour: "pop", paths: ["M16 20v-1.5A3.5 3.5 0 0 0 12.5 15h-5A3.5 3.5 0 0 0 4 18.5V20", "M16 4.4a3.5 3.5 0 0 1 0 6.2", "M20 20v-1.5a3.5 3.5 0 0 0-2.5-3.3"], extra: <circle cx="10" cy="8" r="3.2" /> },
  store: { behaviour: "bounce", paths: ["M4 9 5.5 4h13L20 9", "M4 9a2.7 2.7 0 0 0 5.3 0 2.7 2.7 0 0 0 5.4 0A2.7 2.7 0 0 0 20 9", "M5 12v8h14v-8", "M10 20v-4.5h4V20"] },
  orders: { behaviour: "slide", paths: ["M4 7.5 12 3l8 4.5v9L12 21l-8-4.5z", "M4 7.5 12 12l8-4.5", "M12 12v9"] },
  money: { behaviour: "pulse", paths: ["M3.5 7h17v11h-17z", "M3.5 7 17 4l1 3"], extra: <circle cx="12" cy="12.5" r="2.4" /> },
  alert: { behaviour: "wiggle", paths: ["M6 16V11a6 6 0 1 1 12 0v5l1.5 2h-15z", "M10 20.5a2 2 0 0 0 4 0"] },
  audit: { behaviour: "draw", paths: ["M12 3 4.5 6v5.5c0 4.6 3.1 8 7.5 9.5 4.4-1.5 7.5-4.9 7.5-9.5V6z", "m9 12 2.2 2.2L15.5 10"] },
  settings: { behaviour: "spin", paths: ["M12 2.8v2.6M12 18.6v2.6M2.8 12h2.6M18.6 12h2.6M5.5 5.5l1.9 1.9M16.6 16.6l1.9 1.9M18.5 5.5l-1.9 1.9M7.4 16.6l-1.9 1.9"], extra: <circle cx="12" cy="12" r="3.6" /> },
  dispute: { behaviour: "tilt", paths: ["M12 4v16", "M7 20h10", "M5 7h14", "M5 7l-2.5 6a3 3 0 0 0 5 0z", "M19 7l-2.5 6a3 3 0 0 0 5 0z"] },
  flag: { behaviour: "wiggle", paths: ["M5 21V4", "M5 5c4-2 6 2 10 0v8c-4 2-6-2-10 0"] },
  logout: { behaviour: "slide", paths: ["M10 4H5.5A1.5 1.5 0 0 0 4 5.5v13A1.5 1.5 0 0 0 5.5 20H10", "M15 8l4 4-4 4", "M19 12H9"] },
  search: { behaviour: "tilt", paths: ["m20 20-4.2-4.2"], extra: <circle cx="11" cy="11" r="6.5" /> },
  check: { behaviour: "draw", paths: ["m5 12.5 4.5 4.5L19 7.5"] },
  x: { behaviour: "spin", paths: ["M6 6l12 12M18 6 6 18"] },
  plus: { behaviour: "spin", paths: ["M12 5v14M5 12h14"] },
  refresh: { behaviour: "turn", paths: ["M20 11a8 8 0 0 0-14.5-3.5L4 9", "M4 4v5h5", "M4 13a8 8 0 0 0 14.5 3.5L20 15", "M20 20v-5h-5"] },
  copy: { behaviour: "pop", paths: ["M9 9h10v11H9z", "M15 9V5H5v11h4"] },
  eye: { behaviour: "pulse", paths: ["M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"], extra: <circle cx="12" cy="12" r="2.8" /> },
  ban: { behaviour: "spin", paths: ["M5.6 5.6l12.8 12.8"], extra: <circle cx="12" cy="12" r="8.5" /> },
  truck: { behaviour: "slide", paths: ["M2.5 6.5h11v9h-11z", "M13.5 9.5H18l3 3.2v2.8h-7.5", "M3 15.5v1"], extra: <><circle cx="7" cy="17" r="1.8" /><circle cx="17" cy="17" r="1.8" /></> },
  clock: { behaviour: "turn", paths: ["M12 7.5V12l3 2"], extra: <circle cx="12" cy="12" r="8.5" /> },
  sun: { behaviour: "spin", paths: ["M12 2.5v2.2M12 19.3v2.2M2.5 12h2.2M19.3 12h2.2M5.3 5.3l1.6 1.6M17.1 17.1l1.6 1.6M18.7 5.3l-1.6 1.6M6.9 17.1l-1.6 1.6"], extra: <circle cx="12" cy="12" r="4" /> },
  moon: { behaviour: "tilt", paths: ["M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"] },
  chevron: { behaviour: "bounce", paths: ["m9 6 6 6-6 6"] },
  menu: { behaviour: "slide", paths: ["M4 7h16M4 12h16M4 17h16"] },
  info: { behaviour: "pop", paths: ["M12 11v5.5", "M12 7.6v.1"], extra: <circle cx="12" cy="12" r="8.5" /> },
  warn: { behaviour: "wiggle", paths: ["M12 4 2.8 19.5h18.4z", "M12 10v4.5", "M12 17.2v.1"] },
  undo: { behaviour: "turn", paths: ["M9 8H15a5 5 0 0 1 0 10H8", "M12 4.5 8.5 8 12 11.5"] },
  more: { behaviour: "pop", paths: [], extra: <><circle cx="5.5" cy="12" r="1.5" fill="currentColor" stroke="none" /><circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none" /><circle cx="18.5" cy="12" r="1.5" fill="currentColor" stroke="none" /></> },
  arrow: { behaviour: "slide", paths: ["M5 12h14", "m13 6 6 6-6 6"] },
  external: { behaviour: "tilt", paths: ["M14 4h6v6", "M20 4 10.5 13.5", "M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5"] },
  filter: { behaviour: "pulse", paths: ["M4 5h16l-6 7.5V19l-4 1.5v-8z"] },
  trash: { behaviour: "wiggle", paths: ["M4 7h16", "M9 7V4h6v3", "M6 7l1 13h10l1-13", "M10 11v6M14 11v6"] },
  edit: { behaviour: "tilt", paths: ["M4 20h4L19 9a2.8 2.8 0 0 0-4-4L4 16z", "m13.5 6.5 4 4"] },
  lock: { behaviour: "bounce", paths: ["M6 11h12v9H6z", "M8.5 11V8a3.5 3.5 0 0 1 7 0v3"], extra: <circle cx="12" cy="15.5" r="1.1" fill="currentColor" stroke="none" /> },
  inbox: { behaviour: "bounce", paths: ["M4 13 6.5 5h11L20 13", "M4 13v6h16v-6h-5l-1.5 2.5h-3L9 13z"] },
  command: { behaviour: "pulse", paths: ["M9 9h6v6H9z", "M9 9V6.5A2.5 2.5 0 1 0 6.5 9H9", "M15 9V6.5A2.5 2.5 0 1 1 17.5 9H15", "M9 15v2.5A2.5 2.5 0 1 1 6.5 15H9", "M15 15v2.5a2.5 2.5 0 1 0 2.5-2.5H15"] },
  sparkle: { behaviour: "pop", paths: ["m12 3 1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z", "M19 17v4M17 19h4"] },
  calendar: { behaviour: "bounce", paths: ["M4 6h16v14H4z", "M4 10h16", "M8 3v4M16 3v4"] },
  bolt: { behaviour: "pulse", paths: ["M13 3 5 13.5h6L10 21l8-10.5h-6z"] },
  download: { behaviour: "bounce", paths: ["M12 4v11", "m7 11 5 5 5-5", "M5 20h14"] },
} satisfies Record<string, Glyph>;

export type IconName = keyof typeof glyphs;
/** Every icon, for the kit page. */
export const Glyphs = glyphs;

interface IconProps {
  name: IconName;
  size?: number;
  /** Plays the icon's hover motion once when it turns true (a selected nav item). */
  active?: boolean;
  /** Set false inside a parent that already animates to `hover`/`tap`. */
  interactive?: boolean;
  className?: string;
  title?: string;
}

export function Icon({ name, size = 20, active = false, interactive = true, className, title }: IconProps) {
  const g: Glyph = glyphs[name];
  const variants = keyframes(g.behaviour);
  const drawn = g.behaviour === "draw";
  return (
    <motion.span
      className={`icon ${className ?? ""}`}
      style={{ width: size, height: size }}
      initial="rest"
      animate={active ? "hover" : "rest"}
      whileHover={interactive ? "hover" : undefined}
      whileTap={interactive ? "tap" : undefined}
      role={title ? "img" : undefined}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      <svg
        viewBox="0 0 24 24"
        width={size}
        height={size}
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <motion.g variants={drawn ? undefined : variants} style={{ originX: "50%", originY: "50%", transformBox: "fill-box" }}>
          {g.paths.map((d) => (
            <motion.path key={d} d={d} variants={drawn ? variants : undefined} />
          ))}
          {g.extra}
        </motion.g>
      </svg>
    </motion.span>
  );
}
