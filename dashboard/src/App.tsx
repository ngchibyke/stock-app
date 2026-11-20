import { MotionConfig } from "motion/react";
import { Route, Routes } from "react-router-dom";
import { AuthProvider, RequireAuth, RequireOwner } from "./auth";
import { Shell } from "./Shell";
import { Alerts } from "./pages/Alerts";
import { Audit } from "./pages/Audit";
import { Codes } from "./pages/Codes";
import { Disputes } from "./pages/Disputes";
import { Home } from "./pages/Home";
import { Kit } from "./pages/Kit";
import { Moderation } from "./pages/Moderation";
import { MoneyPage } from "./pages/Money";
import { Orders } from "./pages/Orders";
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
                {/* The kit page is for building the dashboard, so only the dev server has it. */}
                {import.meta.env.DEV && <Route path="kit" element={<Kit />} />}
                <Route path="disputes" element={<Disputes />} />
                <Route path="moderation" element={<Moderation />} />
                <Route element={<RequireOwner />}>
                  <Route path="orders" element={<Orders />} />
                  <Route path="money" element={<MoneyPage />} />
                  <Route path="alerts" element={<Alerts />} />
                  <Route path="audit" element={<Audit />} />
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
