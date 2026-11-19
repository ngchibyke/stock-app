import { type Page, expect, test } from "@playwright/test";
import { OWNER } from "./setup";

test.describe.configure({ mode: "serial" });

// A phone number that is new on every run, because the database is kept between runs.
const staffPhone = `0803${String(Math.floor(Math.random() * 1e7)).padStart(7, "0")}`;
const runId = Date.now();
const STAFF = { name: `Ada Support ${runId}`, password: "support-password-1" };
const label = `E2E codes ${runId}`;

async function signIn(page: Page, phone: string, password: string) {
  await page.goto("/login");
  await page.getByLabel("Phone number").fill(phone);
  await page.getByLabel("Password", { exact: true }).fill(password);
  await page.getByRole("button", { name: "Sign in" }).click();
}

async function signOut(page: Page) {
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByRole("form", { name: "Sign in" })).toBeVisible();
}

test("a wrong password is refused plainly, the right one gets in", async ({ page }) => {
  await signIn(page, OWNER.phone, "not-the-password");
  await expect(page.getByRole("alert")).toContainText("do not match");
  await page.getByLabel("Password", { exact: true }).fill(OWNER.password);
  await page.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("heading", { name: /Good to see you/ })).toBeVisible();
  const nav = page.getByRole("navigation");
  for (const item of ["Overview", "Disputes", "Orders", "Reconciliation", "Alerts", "Access codes", "Users", "Stores", "Settings", "Audit log"]) {
    await expect(nav.getByRole("link", { name: item })).toBeVisible();
  }
  await signOut(page);
  // The session is gone: a protected page sends us back to sign in.
  await page.goto("/settings");
  await expect(page.getByRole("form", { name: "Sign in" })).toBeVisible();
});

test("the owner creates an access code, sees it once, then revokes it", async ({ page }) => {
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Access codes" }).click();
  await page.getByRole("button", { name: "New code" }).click();
  await page.getByLabel("Label (who is it for?)").fill(label);
  await page.getByLabel("Most uses (empty for unlimited)").fill("3");
  await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();

  const reveal = page.locator(".code-reveal");
  await expect(reveal).toHaveText(/^[A-Z2-9]{5}-[A-Z2-9]{5}$/);
  await page.getByRole("button", { name: "Done" }).click();

  const row = page.getByRole("row", { name: new RegExp(label) });
  await expect(row).toContainText("0 / 3");
  await expect(row).toContainText("active");
  await row.getByRole("button", { name: "Revoke" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Revoke" }).click();
  await expect(row).toContainText("revoked");
  await expect(row.getByRole("button", { name: "Revoke" })).toHaveCount(0);
});

test("the owner adds Support, who sees only their own desk", async ({ page }) => {
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Users" }).click();
  await page.getByRole("tab", { name: "Staff" }).click();
  await page.getByRole("button", { name: "Add Support" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Name").fill(STAFF.name);
  await dialog.getByLabel("Phone number").fill(staffPhone);
  await dialog.getByLabel(/First password/).fill(STAFF.password);
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("row", { name: new RegExp(STAFF.name) })).toBeVisible();
  await signOut(page);

  await signIn(page, staffPhone, STAFF.password);
  await expect(page.getByRole("heading", { name: /Good to see you, Ada/ })).toBeVisible();
  const nav = page.getByRole("navigation");
  await expect(nav.getByRole("link", { name: "Disputes" })).toBeVisible();
  await expect(nav.getByRole("link", { name: "Moderation" })).toBeVisible();
  for (const hidden of ["Settings", "Access codes", "Orders", "Audit log", "Users"]) {
    await expect(nav.getByRole("link", { name: hidden })).toHaveCount(0);
  }
  // Typing an owner-only address lands on the disputes desk, not an error.
  await page.goto("/settings");
  await expect(page).toHaveURL(/\/disputes$/);
  await expect(page.getByRole("heading", { name: "Disputes" })).toBeVisible();
  await signOut(page);
});

test("settings save only what changed and survive a reload", async ({ page }) => {
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Settings" }).click();
  const cut = page.getByLabel("Platform cut", { exact: true });
  await expect(cut).toBeVisible();
  const before = await cut.inputValue();
  const next = before === "6" ? "5" : "6";
  await cut.fill(next);
  await expect(page.getByText("1 unsaved change")).toBeVisible();
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText(/Settings saved/)).toBeVisible();
  await expect(page.getByText("unsaved change")).toHaveCount(0);
  await page.reload();
  await expect(page.getByLabel("Platform cut", { exact: true })).toHaveValue(next);
  // Put it back so the run leaves things as it found them.
  await page.getByLabel("Platform cut", { exact: true }).fill(before);
  await page.getByRole("button", { name: "Save changes" }).click();
  await expect(page.getByText(/Settings saved/).first()).toBeVisible();
});

test("a bad value is held back with a reason", async ({ page }) => {
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Settings" }).click();
  const hours = page.getByLabel("Seller ship deadline", { exact: true });
  await hours.fill("soon");
  await expect(page.getByText("fix the highlighted fields first")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save changes" })).toBeDisabled();
  await page.getByRole("button", { name: "Discard" }).click();
  await expect(page.getByText("fix the highlighted fields first")).toHaveCount(0);
});

test("the audit log shows what the owner just did, and opens an entry", async ({ page }) => {
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Audit log" }).click();
  await page.getByLabel("Action", { exact: true }).fill("access_code.created");
  const entry = page.getByRole("button", { name: /access_code\.created/ }).first();
  await expect(entry).toBeVisible();
  await entry.click();
  await expect(page.getByText(/"max_uses": 3/)).toBeVisible();
  await page.getByLabel("Action", { exact: true }).fill("admin.staff_added");
  await expect(page.getByRole("button", { name: /admin\.staff_added/ }).first()).toBeVisible();
});

test("alerts and the overview load without errors", async ({ page }) => {
  const problems: string[] = [];
  page.on("pageerror", (e) => problems.push(e.message));
  await signIn(page, OWNER.phone, OWNER.password);
  await expect(page.getByText("Open items")).toBeVisible();
  await page.getByRole("link", { name: "Alerts" }).click();
  await expect(page.getByRole("heading", { name: "Alerts" })).toBeVisible();
  await page.getByRole("link", { name: "Reconciliation" }).click();
  await expect(page.getByRole("tab", { name: /Payouts/ })).toBeVisible();
  await page.getByRole("tab", { name: "Statements" }).click();
  await expect(page.getByText(/No statement checks yet|Statement checks/)).toBeVisible();
  await page.getByRole("link", { name: "Orders" }).click();
  await expect(page.getByText(/No orders match|Order/).first()).toBeVisible();
  expect(problems).toEqual([]);
});

test("the theme switch changes the look and is remembered", async ({ page }) => {
  await page.goto("/login");
  const html = page.locator("html");
  const first = await html.getAttribute("data-theme");
  await page.getByRole("button", { name: /Switch to (dark|light) theme/ }).click();
  const second = await html.getAttribute("data-theme");
  expect(second).not.toBe(first);
  await page.reload();
  await expect(html).toHaveAttribute("data-theme", second as string);
});

test("with reduced motion on, everything still works", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await signIn(page, OWNER.phone, OWNER.password);
  await page.getByRole("link", { name: "Access codes" }).click();
  await page.getByRole("button", { name: "New code" }).click();
  await expect(page.getByRole("dialog", { name: "New access code" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
});
