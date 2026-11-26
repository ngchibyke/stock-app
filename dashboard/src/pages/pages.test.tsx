import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { type AccessCode, Codes, codeStatus } from "./Codes";
import { Settings, fromDraft, toDraft } from "./Settings";
import { Stores } from "./Stores";
import { Users } from "./Users";

afterEach(() => vi.restoreAllMocks());

const code = (over: Partial<AccessCode> = {}): AccessCode => ({ id: "c1", label: "Wuse traders", last4: "AB12", max_uses: 5, uses: 2, expires_at: null, revoked_at: null, ...over });

describe("Codes", () => {
  it("works out a code's status", () => {
    const now = Date.parse("2026-11-19T00:00:00Z");
    expect(codeStatus(code(), now)).toBe("active");
    expect(codeStatus(code({ revoked_at: "2026-11-01T00:00:00Z" }), now)).toBe("revoked");
    expect(codeStatus(code({ expires_at: "2026-11-18T00:00:00Z" }), now)).toBe("expired");
    expect(codeStatus(code({ uses: 5 }), now)).toBe("used_up");
    expect(codeStatus(code({ max_uses: null, uses: 99 }), now)).toBe("active");
  });

  it("lists codes and revokes one after confirmation", async () => {
    let revoked = false;
    const calls = mockApi({
      "GET /admin/access-codes": () => [200, [code({ revoked_at: revoked ? "2026-11-19T00:00:00Z" : null })]],
      "POST /admin/access-codes/c1/revoke": () => {
        revoked = true;
        return [204];
      },
    });
    renderPage(<Codes />);
    expect(await screen.findByText("Wuse traders")).toBeInTheDocument();
    expect(screen.getByText("2 / 5")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Revoke" }));
    await waitFor(() => expect(calls.some((c) => c.method === "POST" && c.path.endsWith("/revoke"))).toBe(true));
    expect(await screen.findByText("revoked")).toBeInTheDocument();
  });

  it("shows a new code once, after creating it", async () => {
    const calls = mockApi({
      "GET /admin/access-codes": () => [200, []],
      "POST /admin/access-codes": () => [201, { id: "n1", code: "ABCDE-FGHJK" }],
    });
    renderPage(<Codes />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "New code" }));
    await user.type(screen.getByLabelText("Label (who is it for?)"), "Gwarinpa");
    await user.type(screen.getByLabelText("Most uses (empty for unlimited)"), "3");
    await user.click(screen.getByRole("button", { name: "Create" }));
    expect(await screen.findByText("ABCDE-FGHJK")).toBeInTheDocument();
    const post = calls.find((c) => c.method === "POST");
    expect(post?.body).toEqual({ label: "Gwarinpa", max_uses: 3, expires_at: null });
  });
});

describe("Settings", () => {
  it("converts between stored and shown values", () => {
    expect(toDraft("percent", 500)).toBe("5");
    expect(fromDraft("percent", "7.5")).toBe(750);
    expect(toDraft("naira", 150050)).toBe("1500.5");
    expect(fromDraft("naira", "1500.50")).toBe(150050);
    expect(toDraft("clock", 1080)).toBe("18:00");
    expect(fromDraft("clock", "06:30")).toBe(390);
    expect(fromDraft("clock", "bad")).toBeNull();
    expect(fromDraft("int", "2.5")).toBeNull();
    expect(fromDraft("int", "")).toBeNull();
  });

  const base = {
    platform_cut_bps: 500, fee_small_kobo: 10000, fee_large_kobo: 25000, fee_threshold_kobo: 500000, stamp_duty_kobo: 5000,
    stamp_duty_threshold_kobo: 1000000, enabled_methods: ["mono_directpay"], payout_provider: "mono", checkout_ttl_minutes: 30,
    ship_deadline_hours: 48, release_after_hours: 72, backstop_days: 30, payout_settlement_buffer_hours: 24,
    jobs_ship_deadline_enabled: true, delivery_cutoff_minutes: 1080, courier_quote_ttl_minutes: 60, courier_price_tolerance_bps: 500,
    strike_window_days: 30, mute_after_strikes: 3, mute_hours: 24, owner_review_after_strikes: 5, updated_at: "2026-11-19T08:00:00Z",
  };

  it("saves only what changed, with the version it read", async () => {
    const calls = mockApi({
      "GET /admin/settings": () => [200, base],
      "PATCH /admin/settings": (b) => [200, { ...base, ...(b as object), updated_at: "2026-11-19T09:00:00Z" }],
    });
    renderPage(<Settings />);
    const user = userEvent.setup();
    const cut = await screen.findByLabelText("Platform cut");
    expect(screen.queryByRole("region", { name: "Unsaved changes" })).not.toBeInTheDocument();
    await user.clear(cut);
    await user.type(cut, "6");
    expect(await screen.findByText(/unsaved change/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true));
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({ platform_cut_bps: 600, expected_updated_at: "2026-11-19T08:00:00Z" });
  });

  it("blocks saving a bad value and can discard", async () => {
    mockApi({ "GET /admin/settings": () => [200, base] });
    renderPage(<Settings />);
    const user = userEvent.setup();
    const hours = await screen.findByLabelText("Seller ship deadline");
    await user.clear(hours);
    await user.type(hours, "x");
    expect(await screen.findByRole("button", { name: "Save changes" })).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(hours).toHaveValue("48"));
  });

  it("tells the owner when someone else saved first", async () => {
    mockApi({
      "GET /admin/settings": () => [200, base],
      "PATCH /admin/settings": () => [409, { error: { code: "stale_settings", message: "Settings changed" } }],
    });
    renderPage(<Settings />);
    const user = userEvent.setup();
    const cut = await screen.findByLabelText("Platform cut");
    await user.clear(cut);
    await user.type(cut, "8");
    await user.click(await screen.findByRole("button", { name: "Save changes" }));
    expect(await screen.findByText(/Someone else changed the settings/)).toBeInTheDocument();
  });

  it("toggles the ship-deadline switch", async () => {
    mockApi({ "GET /admin/settings": () => [200, base] });
    renderPage(<Settings />);
    const user = userEvent.setup();
    const sw = await screen.findByRole("switch", { name: /Refund and strike/ });
    expect(sw).toHaveAttribute("aria-checked", "true");
    await user.click(sw);
    expect(sw).toHaveAttribute("aria-checked", "false");
    expect(await screen.findByText(/unsaved change/)).toBeInTheDocument();
  });
});

describe("Users", () => {
  const person = { id: "u1", display_name: "Amaka Obi", role: "user", is_seller: true, banned: false, muted: false, deleted: false, created_at: "2026-10-01T00:00:00Z" };
  const detail = { ...person, phone: "+2348031112222", banned_at: null, muted_until: null, strike_count: 2, store_ids: ["s1"], orders_as_buyer: 4 };

  it("searches as you type and opens a person", async () => {
    const calls = mockApi({
      "GET /admin/users": () => [200, { items: [person], next_cursor: null }],
      "GET /admin/users/u1": () => [200, detail],
    });
    renderPage(<Users />);
    const user = userEvent.setup();
    expect(await screen.findByText("Amaka Obi")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Search a phone number or a name"), "amaka");
    await waitFor(() => expect(calls.some((c) => c.query.get("q") === "amaka")).toBe(true));
    await user.click(await screen.findByText("Amaka Obi"));
    expect(await screen.findByText("+2348031112222")).toBeInTheDocument();
  });

  it("bans with a reason", async () => {
    const calls = mockApi({
      "GET /admin/users": () => [200, { items: [person], next_cursor: null }],
      "GET /admin/users/u1": () => [200, detail],
      "POST /admin/moderation/users/u1/ban": () => [200, {}],
    });
    renderPage(<Users />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("Amaka Obi"));
    await user.click(await screen.findByRole("button", { name: "Ban" }));
    const dialog = await screen.findByRole("dialog", { name: "Ban this user?" });
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Repeated scams");
    await user.click(within(dialog).getByRole("button", { name: "Ban" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/ban"))?.body).toEqual({ reason: "Repeated scams" }));
  });

  it("lists staff and adds Support", async () => {
    const calls = mockApi({
      "GET /admin/users": () => [200, { items: [], next_cursor: null }],
      "GET /admin/staff": () => [200, { items: [{ id: "o1", display_name: "Chief", role: "owner", created_at: "2026-10-01T00:00:00Z" }] }],
      "POST /admin/staff": () => [201, { id: "s9" }],
    });
    renderPage(<Users />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Staff" }));
    expect(await screen.findByText("Chief")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add Support" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Ada");
    await user.type(within(dialog).getByLabelText("Phone number"), "08031112222");
    await user.type(within(dialog).getByLabelText(/First password/), "long-enough-1");
    await user.click(within(dialog).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ phone: "08031112222", name: "Ada", password: "long-enough-1" }));
  });
});

describe("Stores", () => {
  const store = { id: "s1", name: "Alpha Threads", slug: "alpha-threads", owner_id: "11111111-2222-3333-4444-555555555555", status: "active", created_at: "2026-10-01T00:00:00Z" };
  const detail = { ...store, suspended: false, suspended_reason: null, suspended_at: null, payout_account_status: "verified", product_count: 3, open_order_count: 2, branches: [], team: [] };

  it("suspends a store with a reason", async () => {
    const calls = mockApi({
      "GET /admin/stores": () => [200, { items: [store], next_cursor: null }],
      "GET /admin/stores/s1": () => [200, detail],
      "POST /admin/stores/s1/suspend": () => [200, { ...detail, suspended: true, status: "suspended" }],
    });
    renderPage(<Stores />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("Alpha Threads"));
    expect(await screen.findByText("Orders in progress")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Suspend store" }));
    const dialog = await screen.findByRole("dialog", { name: "Suspend this store?" });
    expect(within(dialog).getByText(/2 order\(s\) in progress carry on/)).toBeInTheDocument();
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Fraud report");
    await user.click(within(dialog).getByRole("button", { name: "Suspend" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/suspend"))?.body).toEqual({ reason: "Fraud report" }));
  });

  it("shows branches and the team read-only", async () => {
    mockApi({
      "GET /admin/stores": () => [200, { items: [store], next_cursor: null }],
      "GET /admin/stores/s1": () => [
        200,
        {
          ...detail,
          branches: [
            { id: "b1", name: "Wuse", area_council: "amac", closed_at: null, collect_enabled: true },
            { id: "b2", name: "Garki", area_council: null, closed_at: "2026-11-01T00:00:00Z", collect_enabled: false },
          ],
          team: [
            { id: "m1", user_id: "u1", display_name: "Chidi Staff", role: "staff", branch_id: "b1", removed_at: null },
            { id: "m2", user_id: "u2", display_name: "Mary Manager", role: "manager", branch_id: null, removed_at: "2026-11-02T00:00:00Z" },
          ],
        },
      ],
    });
    renderPage(<Stores />);
    await userEvent.setup().click(await screen.findByText("Alpha Threads"));
    const branches = await screen.findByRole("table", { name: "Branches" });
    expect(within(branches).getByText("Wuse")).toBeInTheDocument();
    expect(within(branches).getByText("Garki")).toBeInTheDocument();
    const team = screen.getByRole("table", { name: "Team" });
    expect(within(team).getByText("Chidi Staff")).toBeInTheDocument();
    expect(within(team).getByText("Mary Manager")).toBeInTheDocument();
    expect(within(team).getByText("Wuse")).toBeInTheDocument();
    expect(within(team).getByText("Removed")).toBeInTheDocument();
  });

  it("lists a removed then re-added member twice without a key clash", async () => {
    const errors = vi.spyOn(console, "error").mockImplementation(() => {});
    mockApi({
      "GET /admin/stores": () => [200, { items: [store], next_cursor: null }],
      "GET /admin/stores/s1": () => [
        200,
        {
          ...detail,
          team: [
            { id: "m2", user_id: "u1", display_name: "Chidi Staff", role: "manager", branch_id: null, removed_at: null },
            { id: "m1", user_id: "u1", display_name: "Chidi Staff", role: "staff", branch_id: null, removed_at: "2026-11-02T00:00:00Z" },
          ],
        },
      ],
    });
    renderPage(<Stores />);
    await userEvent.setup().click(await screen.findByText("Alpha Threads"));
    const team = await screen.findByRole("table", { name: "Team" });
    expect(within(team).getAllByText("Chidi Staff")).toHaveLength(2);
    expect(within(team).getByText("Removed")).toBeInTheDocument();
    expect(within(team).getByText("Active")).toBeInTheDocument();
    const clash = errors.mock.calls.some((args) => args.some((a) => String(a).includes("same key")));
    expect(clash).toBe(false);
  });

  it("says so when a store has no branches or team", async () => {
    mockApi({
      "GET /admin/stores": () => [200, { items: [store], next_cursor: null }],
      "GET /admin/stores/s1": () => [200, { ...detail, branches: [], team: [] }],
    });
    renderPage(<Stores />);
    await userEvent.setup().click(await screen.findByText("Alpha Threads"));
    expect(await screen.findByText("No branches")).toBeInTheDocument();
    expect(screen.getByText("No team members")).toBeInTheDocument();
  });

  it("filters by status", async () => {
    const calls = mockApi({ "GET /admin/stores": () => [200, { items: [], next_cursor: null }] });
    renderPage(<Stores />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Suspended" }));
    await waitFor(() => expect(calls.some((c) => c.query.get("status") === "suspended")).toBe(true));
  });
});
