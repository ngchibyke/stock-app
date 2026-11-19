import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { Countdown, type DisputeScreen, Disputes } from "./Disputes";
import { Moderation } from "./Moderation";

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

const future = (hours: number) => new Date(Date.now() + hours * 3_600_000).toISOString();
const past = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString();

const queueRow = (status: string, respondBy = future(10)) => ({ id: "d1", order_id: "aaaaaaaa-1111-2222-3333-444444444444", status, reason: "not_as_described", opened_at: past(5), seller_respond_by: respondBy, items_total_kobo: 1_000_000 });
const screenFor = (status: string, respondBy = past(1), extra: Partial<DisputeScreen> = {}): DisputeScreen => ({
  dispute: { id: "d1", order_id: "aaaaaaaa-1111-2222-3333-444444444444", status, reason: "not_as_described", opened_at: past(60), seller_respond_by: respondBy, seller_missed_deadline: false, buyer_statement: { body: "The zip was torn and stained", photo_keys: ["a", "b"], at: past(59) }, seller_statement: { body: "It left the shop perfect", photo_keys: [], at: past(30) }, return_request: null, decision: null },
  order: { order_id: "aaaaaaaa-1111-2222-3333-444444444444", status: "delivered", items_total_kobo: 1_000_000, delivery_fee_kobo: 70_000, held: true, provider_flagged: false, payout_status: null },
  payment: { checkout_status: "paid", provider: "mono", provider_flagged: false, paid_at: past(100), total_kobo: 1_070_000 },
  ledger: [],
  timeline: [{ key: "paid", label: "Paid", at: past(100), current: false }, { key: "delivered", label: "Delivered", at: past(70), current: true }],
  delivery_events: [],
  handover: { applies: true, issued_at: past(90), wrong_entries: 2, locked_until: null, delivered_with_code: false },
  chat: [{ seq: 1, from: "buyer", body: "Where is my order?", sent_at: past(80) }, { seq: 2, from: "seller", body: "On the way", sent_at: past(79) }],
  flagged: [{ from: "seller", severity: "serious", category: "contact_details", original: "call me on 0803", at: past(78) }],
  ...extra,
});

describe("Countdown", () => {
  it("counts down and says when time is up", () => {
    const { rerender } = render(<Countdown until={future(36.5)} />);
    expect(screen.getByLabelText(/36 hours 29 minutes left|36 hours 30 minutes left/)).toBeInTheDocument();
    rerender(<Countdown until={past(1)} />);
    expect(screen.getByText("deadline passed")).toBeInTheDocument();
  });

  it("ticks each second", async () => {
    vi.useFakeTimers();
    render(<Countdown until={new Date(Date.now() + 61_000).toISOString()} />);
    expect(screen.getByText("0h 01m")).toBeInTheDocument();
    await act(async () => {
      vi.advanceTimersByTime(2000);
    });
    expect(screen.getByText("0h 00m")).toBeInTheDocument();
  });
});

describe("Disputes", () => {
  it("asks the server for the chosen state", async () => {
    const calls = mockApi({ "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }] });
    renderPage(<Disputes />);
    expect(await screen.findByText("Not As Described".toLowerCase().replace(/^./, (c) => c.toUpperCase()), { exact: false })).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("tab", { name: "Decided" }));
    await waitFor(() => expect(calls.some((c) => c.query.get("status") === "decided")).toBe(true));
  });

  it("shows both sides, the chat, and the flagged originals to the Owner", async () => {
    mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_decision")],
    });
    renderPage(<Disputes />);
    await userEvent.setup().click(await screen.findByText(/not as described/i));
    expect(await screen.findByText("The zip was torn and stained")).toBeInTheDocument();
    expect(screen.getByText("It left the shop perfect")).toBeInTheDocument();
    expect(screen.getByText("Where is my order?")).toBeInTheDocument();
    expect(screen.getByText("call me on 0803")).toBeInTheDocument();
    expect(screen.getByText("2 wrong entries")).toBeInTheDocument();
  });

  it("hides the Decide button and the flagged originals from Support", async () => {
    mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_decision", past(1), { flagged: undefined })],
    });
    renderPage(<Disputes />, "/", "support");
    await userEvent.setup().click(await screen.findByText(/not as described/i));
    await screen.findByText("The zip was torn and stained");
    expect(screen.queryByRole("button", { name: "Decide" })).not.toBeInTheDocument();
    expect(screen.getByText("Only the Owner decides.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ask for the item back" })).toBeInTheDocument();
    expect(screen.queryByText(/Owner only/)).not.toBeInTheDocument();
  });

  it("will not let the Owner decide while the seller still has time", async () => {
    mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_seller")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_seller", future(10))],
    });
    renderPage(<Disputes />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Seller's turn" }));
    await user.click(await screen.findByText(/not as described/i));
    expect(await screen.findByRole("button", { name: "Decide" })).toBeDisabled();
    expect(screen.getByText("The seller still has time to respond.")).toBeInTheDocument();
  });

  it("previews a partial refund, validates it, and sends the decision after review", async () => {
    const calls = mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_decision")],
      "POST /admin/disputes/d1/decide": () => [200, {}],
    });
    renderPage(<Disputes />);
    const user = userEvent.setup();
    await user.click(await screen.findByText(/not as described/i));
    await user.click(await screen.findByRole("button", { name: "Decide" }));
    const dialog = await screen.findByRole("dialog", { name: "Decide this dispute" });
    await user.click(within(dialog).getByRole("radio", { name: /Refund part of it/ }));
    const amount = within(dialog).getByLabelText(/Refund amount/);
    await user.type(amount, "10000");
    // The whole item total is not "part of it".
    expect(within(dialog).getByRole("button", { name: "Review" })).toBeDisabled();
    expect(within(dialog).getByText(/Enter an amount between/)).toBeInTheDocument();
    await user.clear(amount);
    await user.type(amount, "4000");
    expect(within(dialog).getByText("₦4,000.00")).toBeInTheDocument();
    expect(within(dialog).getByText("₦6,000.00")).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Reasoning (kept in the audit log)"), "Zip damaged, half is fair");
    await user.click(within(dialog).getByRole("button", { name: "Review" }));
    expect(calls.some((c) => c.method === "POST")).toBe(false);
    expect(within(dialog).getByText(/cannot be undone/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Confirm and pay out" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ decision: "partial_refund", refund_kobo: 400_000, note: "Zip damaged, half is fair" }));
  });

  it("sends a full refund without an amount", async () => {
    const calls = mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_decision")],
      "POST /admin/disputes/d1/decide": () => [200, {}],
    });
    renderPage(<Disputes />);
    const user = userEvent.setup();
    await user.click(await screen.findByText(/not as described/i));
    await user.click(await screen.findByRole("button", { name: "Decide" }));
    const dialog = await screen.findByRole("dialog", { name: "Decide this dispute" });
    expect(within(dialog).getByText("₦10,700.00")).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("Reasoning (kept in the audit log)"), "Not as described");
    await user.click(within(dialog).getByRole("button", { name: "Review" }));
    await user.click(within(dialog).getByRole("button", { name: "Confirm and pay out" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ decision: "full_refund", note: "Not as described" }));
  });

  it("asks for the item back and says who pays", async () => {
    const calls = mockApi({
      "GET /admin/disputes": () => [200, { items: [queueRow("awaiting_decision")] }],
      "GET /admin/disputes/d1": () => [200, screenFor("awaiting_decision")],
      "POST /admin/disputes/d1/request-return": () => [200, {}],
    });
    renderPage(<Disputes />, "/", "support");
    const user = userEvent.setup();
    await user.click(await screen.findByText(/not as described/i));
    await user.click(await screen.findByRole("button", { name: "Ask for the item back" }));
    const dialog = await screen.findByRole("dialog", { name: "Ask for the item back" });
    await user.click(within(dialog).getByRole("radio", { name: /seller pays/ }));
    await user.type(within(dialog).getByLabelText(/Note/), "Wrong colour sent");
    await user.click(within(dialog).getByRole("button", { name: "Request return" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ paid_by: "seller", note: "Wrong colour sent" }));
  });
});

describe("Moderation", () => {
  const flag = { id: "f1", user_id: "11111111-2222-3333-4444-555555555555", surface: "chat", severity: "serious", category: "contact_details", source: "word_filter", status: "open", has_strike: true, created_at: past(2) };

  it("lets the Owner read the original and uphold", async () => {
    const calls = mockApi({
      "GET /admin/moderation/flags": () => [200, { items: [flag], next_cursor: null }],
      "GET /admin/moderation/flags/f1": () => [200, { id: "f1", original: "call me on 08031234567", suggestion: "Let's keep it in the app" }],
      "POST /admin/moderation/flags/f1/resolve": () => [200, {}],
    });
    renderPage(<Moderation />);
    const user = userEvent.setup();
    await user.click(await screen.findByText("Contact Details", { exact: false }));
    expect(await screen.findByText("call me on 08031234567")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Uphold" }));
    const dialog = await screen.findByRole("dialog", { name: "Uphold this flag?" });
    await user.click(within(dialog).getByRole("button", { name: "Uphold" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ action: "uphold" }));
  });

  it("never asks for the original when Support is signed in", async () => {
    const calls = mockApi({
      "GET /admin/moderation/flags": () => [200, { items: [flag], next_cursor: null }],
    });
    renderPage(<Moderation />, "/", "support");
    await userEvent.setup().click(await screen.findByText("Contact Details", { exact: false }));
    expect(await screen.findByText("Only the Owner can read the original text.")).toBeInTheDocument();
    expect(calls.some((c) => c.path === "/admin/moderation/flags/f1")).toBe(false);
  });

  it("filters flags by severity", async () => {
    const calls = mockApi({ "GET /admin/moderation/flags": () => [200, { items: [], next_cursor: null }] });
    renderPage(<Moderation />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Serious" }));
    await waitFor(() => expect(calls.some((c) => c.query.get("severity") === "serious")).toBe(true));
  });

  it("bans from the review queue with a reason, and shows no actions to Support", async () => {
    const queue = { items: [{ user_id: "11111111-2222-3333-4444-555555555555", owner_review_at: past(4), muted_until: null, active_strikes: 5 }] };
    const calls = mockApi({
      "GET /admin/moderation/flags": () => [200, { items: [], next_cursor: null }],
      "GET /admin/moderation/review-queue": () => [200, queue],
      "POST /admin/moderation/users/11111111-2222-3333-4444-555555555555/ban": () => [200, {}],
    });
    const user = userEvent.setup();
    const { unmount } = renderPage(<Moderation />, "/", "support");
    await user.click(await screen.findByRole("tab", { name: "Ban review" }));
    expect(await screen.findByText("11111111")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Ban" })).not.toBeInTheDocument();
    unmount();

    renderPage(<Moderation />);
    await user.click(await screen.findByRole("tab", { name: "Ban review" }));
    await user.click(await screen.findByRole("button", { name: "Ban" }));
    const dialog = await screen.findByRole("dialog");
    await user.type(within(dialog).getByPlaceholderText("Why are you doing this?"), "Repeated scam attempts");
    await user.click(within(dialog).getByRole("button", { name: "Ban" }));
    await waitFor(() => expect(calls.find((c) => c.method === "POST")?.body).toEqual({ reason: "Repeated scam attempts" }));
  });

  it("only enables hide for a real review id", async () => {
    const calls = mockApi({
      "GET /admin/moderation/flags": () => [200, { items: [], next_cursor: null }],
      "POST /admin/reviews/aaaaaaaa-1111-2222-3333-444444444444/hide": () => [200, {}],
    });
    renderPage(<Moderation />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("tab", { name: "Reviews" }));
    const hide = screen.getByRole("button", { name: "Hide" });
    expect(hide).toBeDisabled();
    await user.type(screen.getByLabelText("Review id"), "aaaaaaaa-1111-2222-3333-444444444444");
    expect(hide).toBeEnabled();
    await user.click(hide);
    await waitFor(() => expect(calls.some((c) => c.path.endsWith("/hide"))).toBe(true));
  });
});
