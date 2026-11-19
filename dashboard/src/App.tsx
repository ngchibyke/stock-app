import { MotionConfig } from "motion/react";
import { Route, Routes } from "react-router-dom";
import { AuthProvider, RequireAuth, RequireOwner } from "./auth";
import { Shell } from "./Shell";
import { Alerts } from "./pages/Alerts";
import { Audit } from "./pages/Audit";
import { Codes } from "./pages/Codes";
import { Home } from "./pages/Home";
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
