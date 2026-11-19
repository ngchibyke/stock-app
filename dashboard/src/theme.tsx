import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";
import { spring } from "./motion";
import { Icon } from "./ui/Icon";

type Theme = "light" | "dark";
const KEY = "market.theme";

function initial(): Theme {
  const saved = localStorage.getItem(KEY);
  if (saved === "light" || saved === "dark") return saved;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function useTheme(): [Theme, () => void] {
  const [theme, setTheme] = useState<Theme>(initial);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem(KEY, theme);
  }, [theme]);
  const toggle = useCallback(() => setTheme((t) => (t === "dark" ? "light" : "dark")), []);
  return [theme, toggle];
}

/** The sun and moon swap with a spin. */
export function ThemeToggle() {
  const [theme, toggle] = useTheme();
  return (
    <motion.button
      type="button"
      className="icon-btn"
      aria-label={theme === "dark" ? "Switch to light theme" : "Switch to dark theme"}
      onClick={toggle}
      whileHover={{ scale: 1.1 }}
      whileTap={{ scale: 0.88, rotate: 20 }}
      transition={spring.bouncy}
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.span
          key={theme}
          initial={{ opacity: 0, rotate: -90, scale: 0.4 }}
          animate={{ opacity: 1, rotate: 0, scale: 1 }}
          exit={{ opacity: 0, rotate: 90, scale: 0.4 }}
          transition={{ duration: 0.22 }}
          style={{ display: "inline-flex" }}
        >
          <Icon name={theme === "dark" ? "sun" : "moon"} size={18} interactive={false} />
        </motion.span>
      </AnimatePresence>
    </motion.button>
  );
}
