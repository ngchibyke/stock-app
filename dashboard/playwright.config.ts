import { defineConfig } from "@playwright/test";

// Browser tests run the real server (started in e2e/setup.ts, against the
// Postgres in E2E_DATABASE_URL) and the built dashboard behind `vite preview`.
// No payment, courier, chat-AI or push provider is configured, so these cover
// the owner's own screens: sign-in, codes, staff, settings, users, audit.
const PORT = 4173;
const API_PORT = process.env.E2E_API_PORT ?? "8099";

export default defineConfig({
  testDir: "./e2e",
  globalSetup: "./e2e/setup.ts",
  workers: 1,
  fullyParallel: false,
  timeout: 45_000,
  expect: { timeout: 8_000 },
  reporter: [["list"]],
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "off",
    screenshot: "only-on-failure",
    // CI installs its own browser; locally a preinstalled Chromium can be named.
    launchOptions: process.env.E2E_CHROMIUM ? { executablePath: process.env.E2E_CHROMIUM } : {},
  },
  webServer: {
    command: `npm run build && npx vite preview --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
    env: { VITE_API_TARGET: `http://127.0.0.1:${API_PORT}` },
  },
});
