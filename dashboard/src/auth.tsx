import { type ReactNode, createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { Navigate, Outlet, useLocation } from "react-router-dom";
import { ApiError, api, getToken, request, setToken, setUnauthorizedHandler } from "./api";

export type Role = "owner" | "support" | "user";

export interface Me {
  id: string;
  display_name: string;
  phone: string;
  role: Role;
}

interface AuthState {
  user: Me | null;
  /** True while a stored session is being checked on first load. */
  loading: boolean;
  signIn: (phone: string, password: string) => Promise<void>;
  signOut: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used inside AuthProvider");
  return ctx;
}

const isStaff = (role: Role) => role === "owner" || role === "support";

export function AuthProvider({ children }: { children: ReactNode }) {
  const [user, setUser] = useState<Me | null>(null);
  const [loading, setLoading] = useState<boolean>(() => getToken() !== null);

  const clear = useCallback(() => {
    setToken(null);
    setUser(null);
  }, []);

  useEffect(() => {
    setUnauthorizedHandler(clear);
    return () => setUnauthorizedHandler(null);
  }, [clear]);

  // A session kept from before a reload: check it is still good.
  useEffect(() => {
    if (!getToken()) return;
    let live = true;
    api
      .get<Me>("/me")
      .then((me) => {
        if (!live) return;
        if (isStaff(me.role)) setUser(me);
        else clear();
      })
      .catch(() => live && clear())
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [clear]);

  const signIn = useCallback(
    async (phone: string, password: string) => {
      const { token } = await request<{ token: string }>("POST", "/auth/sign-in", {
        body: { phone, password },
        auth: false,
      });
      setToken(token);
      try {
        const me = await api.get<Me>("/me");
        if (!isStaff(me.role)) {
          // A signed-in shopper or seller has no business here: end that session.
          await request("POST", "/auth/sign-out", {}).catch(() => {});
          clear();
          throw new ApiError(403, "not_staff", "This account is not an Owner or Support account.");
        }
        setUser(me);
      } catch (e) {
        clear();
        throw e;
      }
    },
    [clear],
  );

  const signOut = useCallback(async () => {
    try {
      await request("POST", "/auth/sign-out", { body: {} });
    } catch {
      // The session may already be gone; either way we are out.
    }
    clear();
  }, [clear]);

  const value = useMemo(() => ({ user, loading, signIn, signOut }), [user, loading, signIn, signOut]);
  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/** Everything inside needs a signed-in staff member. */
export function RequireAuth() {
  const { user, loading } = useAuth();
  const location = useLocation();
  if (loading) return null;
  if (!user) return <Navigate to="/login" replace state={{ from: location.pathname }} />;
  return <Outlet />;
}

/** Everything inside needs the Owner; Support is sent to the disputes desk. */
export function RequireOwner() {
  const { user } = useAuth();
  if (user?.role !== "owner") return <Navigate to="/disputes" replace />;
  return <Outlet />;
}
