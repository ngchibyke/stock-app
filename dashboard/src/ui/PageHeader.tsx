import { motion } from "motion/react";
import { type ReactNode } from "react";
import { useLocation } from "react-router-dom";
import { NAV } from "../nav";
import { duration, ease } from "../motion";

/**
 * The top of every screen: the section it belongs to, the title, one line on
 * what the screen is for, and the screen's main actions on the right. Every page
 * uses this, so the rhythm below it is the same everywhere.
 */
export function PageHeader({ title, subtitle, actions }: { title: string; subtitle?: ReactNode; actions?: ReactNode }) {
  const { pathname } = useLocation();
  const group = NAV.find((n) => (n.to === "/" ? pathname === "/" : pathname === n.to || pathname.startsWith(`${n.to}/`)))?.group;
  return (
    <motion.header className="page-head" initial={{ opacity: 0, y: -6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: duration.base, ease: ease.out }}>
      <div className="page-head-text">
        {group && <span className="eyebrow">{group}</span>}
        <h1>{title}</h1>
        {subtitle && <p>{subtitle}</p>}
      </div>
      {actions && <div className="page-actions">{actions}</div>}
    </motion.header>
  );
}
