import { MotionConfig } from "motion/react";
import { Route, Routes } from "react-router-dom";
import { AuthProvider, RequireAuth, RequireOwner } from "./auth";
import { Shell } from "./Shell";
import { Codes } from "./pages/Codes";
import { Home } from "./pages/Home";
import { Settings } from "./pages/Settings";
import { Stores } from "./pages/Stores";
import { Users } from "./pages/Users";
import { Login } from "./pages/Login";
import { ToastProvider } from "./ui/Toasts";

export function App() {
  return (
    // `user`: people who ask their system for less motion get opacity changes only.
    <MotionConfig reducedMotion="user">
      <AuthProvider>
        <ToastProvider>
          <Routes>
            <Route path="/login" element={<Login />} />
            <Route element={<RequireAuth />}>
              <Route element={<Shell />}>
                <Route index element={<Home />} />
                <Route element={<RequireOwner />}>
                  <Route path="codes" element={<Codes />} />
                  <Route path="users" element={<Users />} />
                  <Route path="stores" element={<Stores />} />
                  <Route path="settings" element={<Settings />} />
                </Route>
              </Route>
            </Route>
          </Routes>
        </ToastProvider>
      </AuthProvider>
    </MotionConfig>
  );
}
