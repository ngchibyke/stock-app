import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { IdentityPanel, IdentityReview, type IdentityCheck } from "./Identity";

afterEach(() => vi.restoreAllMocks());

const check = (over: Partial<IdentityCheck> = {}): IdentityCheck => ({
  id: "c1",
  user_id: "11111111-2222-3333-4444-555555555555",
  status: "review",
  reason: "name_mismatch",
  started_at: "2026-11-25T10:00:00Z",
  decided_at: null,
  decided_by: null,
  ...over,
});

describe("Identity review", () => {
  it("shows the queue with the reason and the verified name to the Owner", async () => {
    mockApi({ "GET /admin/identity/review-queue": () => [200, { items: [check({ verified_name: "Ada Obi" })] }] });
    renderPage(<IdentityReview owner />);
    expect(await screen.findByText("name mismatch")).toBeInTheDocument();
    expect(screen.getByText("Ada Obi")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pass" })).toBeInTheDocument();
  });

  it("gives Support the list without a name or any decision button", async () => {
    mockApi({ "GET /admin/identity/review-queue": () => [200, { items: [check()] }] });
    renderPage(<IdentityReview owner={false} />, "/", "support");
    expect(await screen.findByText("name mismatch")).toBeInTheDocument();
    expect(screen.getByText("hidden")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Pass" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Fail" })).toBeNull();
  });

  it("will not pass without a reason, then sends the decision and the reason", async () => {
    const calls = mockApi({
      "GET /admin/identity/review-queue": () => [200, { items: [check()] }],
      "POST /admin/identity/c1/decide": () => [200, { id: "c1" }],
    });
    renderPage(<IdentityReview owner />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Pass" }));
    const dialog = await screen.findByRole("dialog", { name: "Pass this check?" });
    await user.click(within(dialog).getByRole("button", { name: "Pass" }));
    expect(calls.some((c) => c.method === "POST")).toBe(false);
    await user.type(within(dialog).getByRole("textbox"), "Called the seller");
    await user.click(within(dialog).getByRole("button", { name: "Pass" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/decide"))?.body).toEqual({ decision: "pass", reason: "Called the seller" }));
  });

  it("fails a check with a reason", async () => {
    const calls = mockApi({
      "GET /admin/identity/review-queue": () => [200, { items: [check()] }],
      "POST /admin/identity/c1/decide": () => [200, { id: "c1" }],
    });
    renderPage(<IdentityReview owner />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Fail" }));
    const dialog = await screen.findByRole("dialog", { name: "Fail this check?" });
    await user.type(within(dialog).getByRole("textbox"), "Face does not match");
    await user.click(within(dialog).getByRole("button", { name: "Fail" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/decide"))?.body).toEqual({ decision: "fail", reason: "Face does not match" }));
  });
});

describe("Identity panel", () => {
  it("shows a passed person's name and revokes with a reason", async () => {
    const calls = mockApi({
      "GET /admin/users/u1/identity": () => [200, { items: [check({ status: "passed", reason: null, verified_name: "Ada Obi", birth_year: 1990 })] }],
      "POST /admin/identity/c1/revoke": () => [200, { id: "c1" }],
    });
    renderPage(<IdentityPanel userId="u1" owner />);
    const user = userEvent.setup();
    expect(await screen.findByText("Ada Obi")).toBeInTheDocument();
    expect(screen.getByText("1990")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Revoke" }));
    const dialog = await screen.findByRole("dialog", { name: "Revoke this check?" });
    expect(within(dialog).getByText(/ban the account instead/)).toBeInTheDocument();
    await user.type(within(dialog).getByRole("textbox"), "Fraud found");
    await user.click(within(dialog).getByRole("button", { name: "Revoke" }));
    await waitFor(() => expect(calls.find((c) => c.path.endsWith("/revoke"))?.body).toEqual({ reason: "Fraud found" }));
  });

  it("says so when the person has not started", async () => {
    mockApi({ "GET /admin/users/u1/identity": () => [200, { items: [] }] });
    renderPage(<IdentityPanel userId="u1" owner />);
    expect(await screen.findByText("Has not started the check.")).toBeInTheDocument();
  });
});
