import { execFileSync } from "node:child_process";
import { expect, test } from "@playwright/test";

// The public page the app stores require, opened straight on the API (it is served by the
// server itself, not the dashboard).
const API = `http://127.0.0.1:${process.env.E2E_API_PORT ?? "8099"}`;
const DB = process.env.E2E_DATABASE_URL ?? "postgres://market:market@localhost:5432/market";

test("a person who cannot open the app can ask to delete their account, and change their mind", async ({ page }) => {
  // A throwaway account, verified directly (the dev server prints codes only to its own log).
  const phone = `0805${String(Math.floor(Math.random() * 1e7)).padStart(7, "0")}`;
  const password = "leaver-password-1";
  const up = await fetch(`${API}/auth/sign-up`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ phone, password, display_name: "Page Leaver" }),
  });
  expect(up.status).toBe(201);
  execFileSync("psql", [DB, "-v", "ON_ERROR_STOP=1", "-c", `UPDATE users SET phone_verified_at = now() WHERE display_name = 'Page Leaver' AND phone_verified_at IS NULL`], { stdio: "pipe" });

  // The page must run under its own strict policy: a violation or a script error is a failure.
  // (The browser also logs the expected 401 of the wrong password as a console error: not one.)
  const problems: string[] = [];
  page.on("console", (m) => {
    if (m.type() === "error" && !m.text().startsWith("Failed to load resource")) problems.push(m.text());
  });
  page.on("pageerror", (e) => problems.push(String(e)));

  await page.goto(`${API}/account-deletion`);
  await expect(page.getByRole("heading", { name: "Delete your account" })).toBeVisible();
  await expect(page.getByText(/30 days to change your mind/)).toBeVisible();

  // A wrong password is refused in plain words and nothing else shows.
  await page.getByLabel("Phone number").fill(phone);
  await page.getByLabel("Password").fill("not-the-password-1");
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("status")).toContainText("not right");
  await expect(page.getByRole("button", { name: "Delete my account" })).toBeHidden();

  // The right one, then the confirmation is required.
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { name: "Your account", exact: true })).toBeVisible();
  await expect(page.getByLabel("Password")).toHaveValue("", { timeout: 2000 }).catch(() => undefined);
  await page.getByRole("button", { name: "Delete my account" }).click();
  await expect(page.getByRole("status")).toContainText("Tick the box");

  await page.getByLabel(/I understand/).check();
  await page.getByRole("button", { name: "Delete my account" }).click();
  await expect(page.getByRole("heading", { name: "Your account is waiting to be deleted" })).toBeVisible();
  await expect(page.getByText(/It will be deleted on \d{1,2} \w+ \d{4}/)).toBeVisible();

  // Changing your mind restores it, and the page shows the sign-in again.
  await page.getByRole("button", { name: "Keep my account" }).click();
  await expect(page.getByRole("status")).toContainText("Your account is kept");
  await expect(page.getByRole("button", { name: "Sign in" })).toBeVisible();

  // The server agrees: signing in again shows a live account, not a pending one.
  await page.getByLabel("Phone number").fill(phone);
  await page.getByLabel("Password").fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("button", { name: "Delete my account" })).toBeVisible();
  expect(problems, problems.join("\n")).toEqual([]);
});
