// The sidebar's destinations. Kept apart from the shell so the page header and
// the command palette can read the same list.
import { type Role } from "./auth";
import { type IconName } from "./ui/Icon";

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
