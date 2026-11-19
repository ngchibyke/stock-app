import { MotionConfig } from "motion/react";
import { Route, Routes } from "react-router-dom";
import { AuthProvider, RequireAuth } from "./auth";
import { Shell } from "./Shell";
import { Home } from "./pages/Home";
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
              </Route>
            </Route>
          </Routes>
        </ToastProvider>
      </AuthProvider>
    </MotionConfig>
  );
}
