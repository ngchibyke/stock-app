import { motion } from "motion/react";
import { type FormEvent, useState } from "react";
import { Navigate, useLocation, useNavigate } from "react-router-dom";
import { ApiError } from "../api";
import { useAuth } from "../auth";
import { spring } from "../motion";
import { ThemeToggle } from "../theme";
import { Button } from "../ui/Button";
import { Icon } from "../ui/Icon";

export function Login() {
  const { user, loading, signIn } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();
  const [phone, setPhone] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [shake, setShake] = useState(0);
  const [show, setShow] = useState(false);

  const from = (location.state as { from?: string } | null)?.from ?? "/";
  if (!loading && user) return <Navigate to={from} replace />;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setBusy(true);
    try {
      await signIn(phone.trim(), password);
      navigate(from, { replace: true });
    } catch (err) {
      setError(
        err instanceof ApiError && err.code === "invalid_credentials"
          ? "That phone number and password do not match."
          : err instanceof Error
            ? err.message
            : "Could not sign in.",
      );
      setShake((s) => s + 1);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login">
      <motion.div className="login-orb" style={{ width: 420, height: 420, background: "var(--accent)", top: -120, left: -100 }} animate={{ x: [0, 40, 0], y: [0, 30, 0] }} transition={{ repeat: Infinity, duration: 14, ease: "easeInOut" }} />
      <motion.div className="login-orb" style={{ width: 360, height: 360, background: "var(--accent-2)", bottom: -120, right: -80 }} animate={{ x: [0, -40, 0], y: [0, -30, 0] }} transition={{ repeat: Infinity, duration: 17, ease: "easeInOut" }} />
      <div style={{ position: "absolute", top: 16, right: 16 }}>
        <ThemeToggle />
      </div>
      <motion.form
        className="card login-card"
        onSubmit={submit}
        initial={{ opacity: 0, y: 24, scale: 0.97 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={spring.gentle}
        aria-label="Sign in"
      >
        <motion.div className="brand-mark" style={{ width: 44, height: 44, borderRadius: 13 }} whileHover={{ rotate: 14, scale: 1.1 }} whileTap={{ scale: 0.9 }} transition={spring.bouncy}>
          <Icon name="store" size={24} interactive={false} />
        </motion.div>
        <h1>Owner dashboard</h1>
        <p className="muted">Sign in with your Owner or Support account.</p>
        <motion.div key={shake} animate={shake ? { x: [0, -9, 9, -6, 6, 0] } : undefined} transition={{ duration: 0.4 }}>
          <label className="field">
            <span>Phone number</span>
            <input value={phone} onChange={(e) => setPhone(e.target.value)} inputMode="tel" autoComplete="username" placeholder="0803 123 4567" required />
          </label>
          <label className="field">
            <span>Password</span>
            <div style={{ position: "relative" }}>
              <input type={show ? "text" : "password"} value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required style={{ paddingRight: 42 }} />
              <button type="button" className="icon-btn" aria-label={show ? "Hide password" : "Show password"} style={{ position: "absolute", right: 3, top: 2 }} onClick={() => setShow((s) => !s)}>
                <Icon name="eye" size={18} active={show} interactive={false} />
              </button>
            </div>
          </label>
        </motion.div>
        {error && (
          <motion.div className="error-note" role="alert" style={{ marginTop: 14 }} initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto" }}>
            <Icon name="warn" size={18} active />
            <span>{error}</span>
          </motion.div>
        )}
        <div style={{ marginTop: 18 }}>
          <Button type="submit" variant="primary" loading={busy} disabled={!phone || !password} style={{ width: "100%" }}>
            Sign in
          </Button>
        </div>
      </motion.form>
    </div>
  );
}
