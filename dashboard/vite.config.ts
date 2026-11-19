/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// In development the API runs on :8080 (the server's default BIND_ADDR). The
// server has no CORS: in production serve this build and the API from one origin.
const api = process.env.VITE_API_TARGET ?? "http://localhost:8080";

const proxy = { "/auth": api, "/me": api, "/admin": api };

export default defineConfig({
  plugins: [react()],
  server: { proxy },
  // `vite preview` serves the built app for the browser tests, with the same proxy.
  preview: { proxy },
  test: {
    exclude: ["e2e/**", "node_modules/**"],
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test/setup.ts"],
    css: false,
  },
});
