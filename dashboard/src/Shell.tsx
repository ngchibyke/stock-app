// The app frame: a collapsible sidebar whose active item is a pill that slides
// between links, icons that animate on hover, and a page that rises in on each
// route change.
import { useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useState } from "react";
import { NavLink, Outlet, useLocation } from "react-router-dom";
import { api } from "./api";
import { type Role, useAuth } from "./auth";
import { pageVariants, spring } from "./motion";
import { ThemeToggle } from "./theme";
import { Icon, type IconName } from "./ui/Icon";
import { IconButton } from "./ui/Button";

export interface NavItem {
  to: string;
  label: string;
  icon: IconName;
  roles: Role[];
  group: string;
}

/** Screens add themselves here as they are built. */
export const NAV: NavItem[] = [
  { to: "/", label: "Overview", icon: "home", roles: ["owner", "support"], group: "Market" },
  { to: "/disputes", label: "Disputes", icon: "dispute", roles: ["owner", "support"], group: "Resolve" },
  { to: "/moderation", label: "Moderation", icon: "flag", roles: ["owner", "support"], group: "Resolve" },
  { to: "/orders", label: "Orders", icon: "orders", roles: ["owner"], group: "Money" },
  { to: "/money", label: "Reconciliation", icon: "money", roles: ["owner"], group: "Money" },
  { to: "/alerts", label: "Alerts", icon: "alert", roles: ["owner"], group: "Money" },
  { to: "/codes", label: "Access codes", icon: "key", roles: ["owner"], group: "People" },
  { to: "/users", label: "Users", icon: "users", roles: ["owner"], group: "People" },
  { to: "/stores", label: "Stores", icon: "store", roles: ["owner"], group: "People" },
  { to: "/settings", label: "Settings", icon: "settings", roles: ["owner"], group: "Configure" },
  { to: "/audit", label: "Audit log", icon: "audit", roles: ["owner"], group: "Configure" },
];

function Link({ item, collapsed, badge }: { item: NavItem; collapsed: boolean; badge?: number }) {
  return (
    <NavLink to={item.to} end={item.to === "/"} className={({ isActive }) => `nav-link ${isActive ? "on" : ""}`} title={collapsed ? item.label : undefined}>
      {({ isActive }) => (
        <motion.span style={{ display: "contents" }} initial="rest" whileHover="hover" whileTap="tap">
          {isActive && <motion.span layoutId="nav-pill" className="nav-pill" transition={spring.snappy} />}
          <Icon name={item.icon} size={20} active={isActive} interactive={false} />
          <motion.span
            animate={{ opacity: collapsed ? 0 : 1, x: collapsed ? -6 : 0 }}
            transition={{ duration: 0.18 }}
            aria-hidden={collapsed}
          >
            {item.label}
          </motion.span>
          <AnimatePresence>
            {!!badge && badge > 0 && (
              <motion.span className="nav-badge" initial={{ scale: 0 }} animate={{ scale: 1 }} exit={{ scale: 0 }} transition={spring.bouncy} aria-label={`${badge} open`}>
                {badge > 99 ? "99+" : badge}
              </motion.span>
            )}
          </AnimatePresence>
        </motion.span>
      )}
    </NavLink>
  );
}

export function Shell() {
  const { user, signOut } = useAuth();
  const location = useLocation();
  const [collapsed, setCollapsed] = useState(false);
  const owner = user?.role === "owner";
  // The sidebar's alert count: refreshed every minute and after every acknowledgement.
  const alerts = useQuery({
    queryKey: ["alerts", "open"],
    queryFn: () => api.get<{ items: unknown[] }>("/admin/alerts", { state: "open", limit: 100 }),
    enabled: owner,
    refetchInterval: 60_000,
  });
  if (!user) return null;
  const items = NAV.filter((n) => n.roles.includes(user.role));
  const groups = [...new Set(items.map((n) => n.group))];

  return (
    <div className="shell">
      <motion.aside className="side" animate={{ width: collapsed ? 68 : 236 }} transition={spring.snappy} aria-label="Main">
        <div className="brand">
          <motion.span className="brand-mark" whileHover={{ rotate: 12, scale: 1.08 }} whileTap={{ scale: 0.9 }} transition={spring.bouncy}>
            <Icon name="store" size={18} interactive={false} />
          </motion.span>
          {!collapsed && <span>Market</span>}
        </div>
        <nav>
          {groups.map((g) => (
            <div key={g}>
              {!collapsed && <div className="nav-group">{g}</div>}
              {items
                .filter((n) => n.group === g)
                .map((n) => (
                  <Link key={n.to} item={n} collapsed={collapsed} badge={n.to === "/alerts" ? alerts.data?.items.length : undefined} />
                ))}
            </div>
          ))}
        </nav>
        <div className="side-foot">
          <div className="row" style={{ justifyContent: collapsed ? "center" : "space-between" }}>
            <ThemeToggle />
            <IconButton icon="menu" label={collapsed ? "Expand menu" : "Collapse menu"} onClick={() => setCollapsed((c) => !c)} />
          </div>
          <div className="me">
            <span className="avatar">{user.display_name.slice(0, 1).toUpperCase()}</span>
            {!collapsed && (
              <span style={{ minWidth: 0 }}>
                <strong style={{ display: "block", overflow: "hidden", textOverflow: "ellipsis" }}>{user.display_name}</strong>
                <span className="muted" style={{ textTransform: "capitalize", fontSize: 12 }}>
                  {user.role}
                </span>
              </span>
            )}
          </div>
          <motion.button
            type="button"
            className="nav-link"
            style={{ background: "none", border: 0, cursor: "pointer", font: "inherit", fontWeight: 600 }}
            onClick={() => void signOut()}
            initial="rest"
            whileHover="hover"
            whileTap="tap"
          >
            <Icon name="logout" size={20} interactive={false} />
            {!collapsed && <span>Sign out</span>}
          </motion.button>
        </div>
      </motion.aside>
      <main className="main">
        {/* Enter-only on purpose: an exit animation (AnimatePresence mode="wait") recreated the page
            under the person's hands and threw away unsaved form state. */}
        <motion.div key={location.pathname} variants={pageVariants} initial="initial" animate="enter">
          <Outlet />
        </motion.div>
      </main>
    </div>
  );
}
