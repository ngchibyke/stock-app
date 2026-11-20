// The app frame: a collapsible sidebar whose active item is a pill that slides
// between links, icons that animate on hover, and a page that rises in on each
// route change.
import { useQuery } from "@tanstack/react-query";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";
import { api } from "./api";
import { useAuth } from "./auth";
import { useMediaQuery } from "./hooks";
import { NAV, type NavItem } from "./nav";
import { pageVariants, spring } from "./motion";
import { ThemeToggle, useTheme } from "./theme";
import { Icon } from "./ui/Icon";
import { IconButton } from "./ui/Button";
import { type Command, CommandPalette } from "./ui/CommandPalette";

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
  // Phones and small tablets get a top bar and a sidebar that slides over the page.
  const mobile = useMediaQuery("(max-width: 900px)");
  const [navOpen, setNavOpen] = useState(false);
  // Mid-size screens start with the slim rail so the page keeps its room.
  const [collapsed, setCollapsed] = useState(() => typeof window !== "undefined" && !!window.matchMedia?.("(max-width: 1100px)").matches);
  const owner = user?.role === "owner";
  const navigate = useNavigate();
  const [, toggleTheme] = useTheme();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const closePalette = useCallback(() => setPaletteOpen(false), []);
  // Ctrl or ⌘ + K opens the command palette from anywhere.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  const commands = useMemo<Command[]>(() => {
    if (!user) return [];
    const go = NAV.filter((n) => n.roles.includes(user.role)).map<Command>((n) => ({ id: `go-${n.to}`, label: n.label, group: `Go to · ${n.group}`, icon: n.icon, run: () => navigate(n.to) }));
    return [
      ...go,
      { id: "theme", label: "Switch light or dark theme", group: "Actions", icon: "sparkle", keywords: "appearance mode colour", run: toggleTheme },
      { id: "signout", label: "Sign out", group: "Actions", icon: "logout", keywords: "log out leave", run: () => void signOut() },
    ];
  }, [user, navigate, toggleTheme, signOut]);
  // The sidebar's alert count: refreshed every minute and after every acknowledgement.
  const alerts = useQuery({
    queryKey: ["alerts", "open"],
    queryFn: () => api.get<{ items: unknown[] }>("/admin/alerts", { state: "open", limit: 100 }),
    enabled: owner,
    refetchInterval: 60_000,
  });

  // Going to another screen, or leaving the phone layout, closes the sliding menu.
  useEffect(() => setNavOpen(false), [location.pathname, mobile]);
  useEffect(() => {
    if (!navOpen) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setNavOpen(false);
    document.addEventListener("keydown", onKey);
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = overflow;
    };
  }, [navOpen]);

  if (!user) return null;
  const items = NAV.filter((n) => n.roles.includes(user.role));
  const groups = [...new Set(items.map((n) => n.group))];
  // On a phone the menu is always the full one: there is no slim rail.
  const slim = collapsed && !mobile;
  const hidden = mobile && !navOpen;

  return (
    <div className="shell">
      {mobile && (
        <header className="topbar">
          <IconButton icon="menu" label="Open menu" onClick={() => setNavOpen(true)} />
          <span className="brand-mark" aria-hidden="true">
            <Icon name="store" size={16} interactive={false} />
          </span>
          <strong className="topbar-title">Market</strong>
          <span style={{ marginLeft: "auto" }}>
            <IconButton icon="search" label="Search" onClick={() => setPaletteOpen(true)} />
          </span>
          <span className="avatar" aria-hidden="true">
            {user.display_name.slice(0, 1).toUpperCase()}
          </span>
        </header>
      )}
      <AnimatePresence>
        {mobile && navOpen && (
          <motion.div className="scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.18 }} onClick={() => setNavOpen(false)} />
        )}
      </AnimatePresence>
      <motion.aside
        className={`side ${mobile ? "side-mobile" : ""}`}
        initial={false}
        animate={mobile ? { x: navOpen ? 0 : "-105%", width: 296 } : { x: 0, width: slim ? 68 : 236 }}
        transition={spring.snappy}
        aria-label="Main"
        aria-hidden={hidden || undefined}
        inert={hidden || undefined}
        data-hidden={hidden || undefined}
      >
        <div className="brand">
          <motion.span className="brand-mark" whileHover={{ rotate: 12, scale: 1.08 }} whileTap={{ scale: 0.9 }} transition={spring.bouncy}>
            <Icon name="store" size={18} interactive={false} />
          </motion.span>
          {!slim && <span>Market</span>}
          {mobile && (
            <span style={{ marginLeft: "auto" }}>
              <IconButton icon="x" label="Close menu" onClick={() => setNavOpen(false)} />
            </span>
          )}
        </div>
        <button type="button" className="side-search" onClick={() => setPaletteOpen(true)} aria-label="Search screens and actions" title={slim ? "Search" : undefined}>
          <Icon name="search" size={16} interactive={false} />
          {!slim && (
            <>
              <span>Search</span>
              <kbd className="kbd">{/Mac|iPhone|iPad/.test(navigator.platform) ? "⌘ K" : "Ctrl K"}</kbd>
            </>
          )}
        </button>
        <nav>
          {groups.map((g) => (
            <div key={g}>
              {!slim && <div className="nav-group">{g}</div>}
              {items
                .filter((n) => n.group === g)
                .map((n) => (
                  <Link key={n.to} item={n} collapsed={slim} badge={n.to === "/alerts" ? alerts.data?.items.length : undefined} />
                ))}
            </div>
          ))}
        </nav>
        <div className="side-foot">
          <div className="row" style={{ justifyContent: slim ? "center" : "space-between" }}>
            <ThemeToggle />
            {!mobile && <IconButton icon="menu" label={slim ? "Expand menu" : "Collapse menu"} onClick={() => setCollapsed((c) => !c)} />}
          </div>
          <div className="me">
            <span className="avatar">{user.display_name.slice(0, 1).toUpperCase()}</span>
            {!slim && (
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
            {!slim && <span>Sign out</span>}
          </motion.button>
        </div>
      </motion.aside>
      <CommandPalette open={paletteOpen} onClose={closePalette} commands={commands} />
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
