import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { Alerts, describe as describeAlert } from "./Alerts";
import { Audit } from "./Audit";
import { MoneyPage } from "./Money";
import { type OrderDetail, Orders } from "./Orders";

afterEach(() => vi.restoreAllMocks());

const order = { order_id: "aaaaaaaa-1111-2222-3333-444444444444", store_id: "s1", status: "delivered", items_total_kobo: 1_000_000, delivery_fee_kobo: 70_000, held: false, paid_at: "2026-11-10T10:00:00Z", created_at: "2026-11-10T09:00:00Z" };
const detail = (over: Partial<OrderDetail["order"]> = {}, extra: Partial<OrderDetail> = {}): OrderDetail => ({
  order: { ...order, platform_cut_bps: 500, platform_cut_kobo: 50_000, collection_fee_share_kobo: 0, hold_reason: null, delivered_at: "2026-11-12T10:00:00Z", completed_at: null, cancelled_at: null, ...over },
  items: [{ product_name: "Ankara bag", attributes: {}, unit_price_kobo: 1_000_000, quantity: 1 }],
  payment: { reference: "CK123", method: "mono_directpay", provider: "mono", status: "paid", provider_flagged: false },
  delivery_events: [{ source: "seller", status: "picked_up", occurred_at: null, received_at: "2026-11-11T10:00:00Z" }],
  refund: null,
  payout: null,
  dispute: null,
  ledger: {
    postings: [{ txn_id: "t1", kind: "payment", ref: "CK123", posted_at: "2026-11-10T10:00:00Z", entries: [{ account: "buyer_funds", amount_kobo: -1_070_000, kind: "payment" }, { account: "escrow", amount_kobo: 1_070_000, kind: "payment" }], sum_kobo: 0 }],
    balances: [{ account: "escrow", kobo: 1_070_000 }, { account: "buyer_funds", kobo: -1_070_000 }],
    balance_kobo: 0,
  },
  ...extra,
});

describe("Orders", () => {
  it("lists orders and sends the filters", async () => {
    const calls = mockApi({ "GET /admin/orders": () => [200, { items: [order], next_cursor: null }] });
    renderPage(<Orders />);
    expect(await screen.findByText("aaaaaaaa")).toBeInTheDocument();
    await userEvent.setup().selectOptions(screen.getByLabelText("Status"), "paid");
    await waitFor(() => expect(calls.some((c) => c.query.get("status") === "paid")).toBe(true));
  });

  it("shows the exact split before releasing, then sends the note", async () => {
    const calls = mockApi({
      "GET /admin/orders": () => [200, { items: [order], next_cursor: null }],
      "GET /admin/orders/aaaaaaaa-1111-2222-3333-444444444444": () => [200, detail()],
      "POST /admin/orders/aaaaaaaa-1111-2222-3333-444444444444/release": () => [200, {}],
    });
    renderPage(<Orders />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("aaaaaaaa"));
    await user.click(await screen.findByRole("button", { name: "Release" }));
    const dialog = await screen.findByRole("dialog", { name: "Release to the seller?" });
    // Items ₦10,000.00 less the 5% cut ₦500.00 leaves ₦9,500.00.
    expect(within(dialog).getByText("₦9,500.00")).toBeInTheDocument();
    expect(within(dialog).getByText("₦500.00")).toBeInTheDocument();
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Buyer confirmed by phone");
    await user.click(within(dialog).getByRole("button", { name: "Release now" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/release"))?.body).toEqual({ note: "Buyer confirmed by phone" }));
  });

  it("offers a refund only once the order is held, and asks for a hold reason first", async () => {
    mockApi({
      "GET /admin/orders": () => [200, { items: [order], next_cursor: null }],
      "GET /admin/orders/aaaaaaaa-1111-2222-3333-444444444444": () => [200, detail()],
    });
    renderPage(<Orders />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("aaaaaaaa"));
    await screen.findByRole("button", { name: "Hold" });
    expect(screen.queryByRole("button", { name: "Refund" })).not.toBeInTheDocument();
    expect(screen.getByText("To refund, hold the order first.")).toBeInTheDocument();
  });

  it("shows Refund and the full amount for a held order", async () => {
    mockApi({
      "GET /admin/orders": () => [200, { items: [{ ...order, held: true }], next_cursor: null }],
      "GET /admin/orders/aaaaaaaa-1111-2222-3333-444444444444": () => [200, detail({ held: true, hold_reason: "complaint" })],
    });
    renderPage(<Orders />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("aaaaaaaa"));
    await user.click(await screen.findByRole("button", { name: "Refund" }));
    const dialog = await screen.findByRole("dialog", { name: "Refund the buyer in full?" });
    expect(within(dialog).getByText("₦10,700.00")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Lift hold" })).toBeInTheDocument();
  });

  it("sends the owner to the dispute screen instead of offering money actions", async () => {
    mockApi({
      "GET /admin/orders": () => [200, { items: [order], next_cursor: null }],
      "GET /admin/orders/aaaaaaaa-1111-2222-3333-444444444444": () => [200, detail({ held: true }, { dispute: { id: "d1", status: "awaiting_decision" } })],
    });
    renderPage(<Orders />);
    await userEvent.setup().click(await screen.findByText("aaaaaaaa"));
    expect(await screen.findByText(/dispute decides the money/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Release" })).not.toBeInTheDocument();
  });

  it("shows the ledger as balanced and opens a posting", async () => {
    mockApi({
      "GET /admin/orders": () => [200, { items: [order], next_cursor: null }],
      "GET /admin/orders/aaaaaaaa-1111-2222-3333-444444444444": () => [200, detail()],
    });
    renderPage(<Orders />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("aaaaaaaa"));
    expect(await screen.findByText("balanced")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /payment/i }));
    expect((await screen.findAllByText("buyer funds")).length).toBeGreaterThan(1);
  });
});

const payout = { id: "p1", order_id: "bbbbbbbb-1111-2222-3333-444444444444", store_id: "s1", reference: "PO1", status: "manual", amount_kobo: 950_000, fee_kobo: 3000, attempts: 3, last_error: "unavailable", account_last4: "6785", reversed: false, settled_by: null, updated_at: "2026-11-15T10:00:00Z" };
const emptyRec = { payouts: [], refunds: [], flagged_checkouts: [], mismatched_checkouts: [], blocked_payouts: [], shipments_attention: [], orders_attention: [] };

describe("Money", () => {
  it("will not mark a payout paid without a provider reference, then sends exactly the right keys", async () => {
    const calls = mockApi({
      "GET /admin/reconciliation": () => [200, { ...emptyRec, payouts: [payout] }],
      "POST /admin/payouts/p1/mark-paid": () => [200, {}],
    });
    renderPage(<MoneyPage />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Paid by hand" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("₦9,500.00")).toBeInTheDocument();
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Paid from the bank app");
    // Blocked, and it says why, until the reference is entered.
    expect(within(dialog).getByRole("button", { name: "Mark paid" })).toBeDisabled();
    expect(within(dialog).getByRole("note")).toHaveTextContent("provider reference");
    expect(calls.some((c) => c.method === "POST")).toBe(false);
    await user.type(within(dialog).getByPlaceholderText("e.g. TRX_8H2KD9"), "TRX_ABC123");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Mark paid" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ note: "Paid from the bank app", provider_reference: "TRX_ABC123", attest_provider_checked: true }));
  });

  it("asks the provider about a payout", async () => {
    const calls = mockApi({
      "GET /admin/reconciliation": () => [200, { ...emptyRec, payouts: [payout] }],
      "POST /admin/payouts/p1/repoll": () => [200, { id: "p1", status: "failed", answer: "settled" }],
    });
    renderPage(<MoneyPage />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Ask provider" }));
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/repoll"))).toBe(true));
  });

  it("re-books an unknown booking only with the attestation", async () => {
    const calls = mockApi({
      "GET /admin/reconciliation": () => [200, { ...emptyRec, shipments_attention: [{ alert_key: "shipment:s1:booking_unknown:", id: "s1", order_id: "cccccccc-1111-2222-3333-444444444444", status: "booking_unknown", order_status: "paid", provider_order_id: null, quoted_kobo: 150000, charged_kobo: null, attempts: 1, last_error: "timed out", attention: null, price_above_quote: false, updated_at: "2026-11-15T10:00:00Z" }] }],
      "POST /admin/shipments/s1/rebook": () => [200, {}],
    });
    renderPage(<MoneyPage />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: /Shipments/ }));
    await user.click(await screen.findByRole("button", { name: "Re-book" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "No label on the dashboard");
    await user.click(within(dialog).getByRole("checkbox"));
    await user.click(within(dialog).getByRole("button", { name: "Book again" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ note: "No label on the dashboard", confirm_not_booked: true }));
  });

  it("shows counts on the tabs and an all-clear when empty", async () => {
    mockApi({ "GET /admin/reconciliation": () => [200, { ...emptyRec, refunds: [{ id: "r1", order_id: "dddddddd-1111-2222-3333-444444444444", reference: "RF1", status: "manual", amount_kobo: 1000, attempts: 2, last_error: null, updated_at: "2026-11-15T10:00:00Z" }] }] });
    renderPage(<MoneyPage />);
    expect(await screen.findByText("No payouts waiting")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /Refunds/ })).toHaveTextContent("1");
  });
});

describe("Alerts", () => {
  const a = { alert_key: "audit:7", kind: "payout.amount_mismatch", subject_id: "p1", at: new Date().toISOString(), detail: { expected_kobo: 900_000, reported_kobo: 800_000 }, acked_by: null, acked_at: null, note: null };

  it("describes the numbers in plain words", () => {
    expect(describeAlert(a)).toBe("We sent ₦9,000.00, the provider says ₦8,000.00.");
    expect(describeAlert({ ...a, kind: "order.repeated_handover_lockout", detail: { lockouts: 3 } })).toBe("3 lockouts so far.");
  });

  it("acknowledges with a note and the card leaves", async () => {
    let open = [a];
    const calls = mockApi({
      "GET /admin/alerts": (_b, url) => [200, { items: url.searchParams.get("state") === "open" ? open : [] }],
      "POST /admin/alerts/ack": () => {
        open = [];
        return [200, {}];
      },
    });
    renderPage(<Alerts />);
    const user = userEvent.setup();
    expect(await screen.findByText("Payout amount differs")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Acknowledge" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Mono confirmed a fee");
    await user.click(within(dialog).getByRole("button", { name: "Acknowledge" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ alert_key: "audit:7", note: "Mono confirmed a fee" }));
    expect(await screen.findByText("All clear")).toBeInTheDocument();
  });
});

describe("Audit", () => {
  const row = { id: 9, at: "2026-11-19T10:00:00Z", actor_id: "11111111-2222-3333-4444-555555555555", action: "admin.store_suspended", subject_type: "store", subject_id: "22222222-2222-3333-4444-555555555555", detail: { reason: "Fraud" } };

  it("filters by action and opens an entry to show its detail", async () => {
    const calls = mockApi({ "GET /admin/audit": () => [200, { items: [row], next_cursor: null }] });
    renderPage(<Audit />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: /admin.store_suspended/ }));
    expect(await screen.findByText(/"reason": "Fraud"/)).toBeInTheDocument();
    await user.type(screen.getByLabelText("Action"), "admin.store_suspended");
    await waitFor(() => expect(calls.some((c) => c.query.get("action") === "admin.store_suspended")).toBe(true));
  });

  it("only sends a full actor id", async () => {
    const calls = mockApi({ "GET /admin/audit": () => [200, { items: [], next_cursor: null }] });
    renderPage(<Audit />);
    const user = userEvent.setup();
    await user.type(await screen.findByLabelText("Actor id"), "1111");
    await new Promise((r) => setTimeout(r, 400));
    expect(calls.every((c) => !c.query.get("actor_id"))).toBe(true);
    await user.clear(screen.getByLabelText("Actor id"));
    await user.type(screen.getByLabelText("Actor id"), "11111111-2222-3333-4444-555555555555");
    await waitFor(() => expect(calls.some((c) => c.query.get("actor_id") === "11111111-2222-3333-4444-555555555555")).toBe(true));
  });
});
