import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { PartnerCard, errorWords, type PartnerStatus } from "./Partner";

const base: PartnerStatus = {
  configured: true,
  enabled: true,
  connected: true,
  breaker: "closed",
  features: [{ name: "price_history", on: true }],
  monthly_call_budget: 1000,
  calls_this_month: 250,
  usage: [{ feature: "price_history", calls: 250, errors: 3 }],
  last_error: null,
};

describe("PartnerCard", () => {
  it("says plainly when no partner is connected, and offers no switches", async () => {
    mockApi({ "GET /admin/partner/status": () => [200, { ...base, configured: false, enabled: false, connected: null, features: [], usage: [] }] });
    renderPage(<PartnerCard />);
    expect(await screen.findByText(/No partner is connected/)).toBeInTheDocument();
    expect(screen.queryByRole("switch")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save partner settings" })).not.toBeInTheDocument();
  });

  it("shows the connection, the usage against the budget and the features", async () => {
    mockApi({ "GET /admin/partner/status": () => [200, base] });
    renderPage(<PartnerCard />);
    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(screen.getByText(/250 calls of 1000/)).toBeInTheDocument();
    expect(screen.getByText(/price_history: 250 calls, 3 errors/)).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Feature price_history" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("progressbar", { name: "Calls used this month" })).toHaveAttribute("value", "250");
    expect(screen.getByText("Calls are going through normally.")).toBeInTheDocument();
  });

  it("says when the partner is not reachable, when calls are paused and why, in words", async () => {
    mockApi({ "GET /admin/partner/status": () => [200, { ...base, connected: false, breaker: "open", last_error: "unavailable" }] });
    renderPage(<PartnerCard />);
    expect(await screen.findByText("Not reachable")).toBeInTheDocument();
    expect(screen.getByText(/Paused: the partner failed several times/)).toBeInTheDocument();
    expect(screen.getByText(/did not answer in time/)).toBeInTheDocument();
    // Never the raw code.
    expect(screen.queryByText("unavailable")).not.toBeInTheDocument();
  });

  it("shows no budget as no cap, and an empty month", async () => {
    mockApi({ "GET /admin/partner/status": () => [200, { ...base, monthly_call_budget: null, calls_this_month: 0, usage: [], features: [] }] });
    renderPage(<PartnerCard />);
    expect(await screen.findByText(/0 calls \(no cap\)/)).toBeInTheDocument();
    expect(screen.getByText("No calls yet this month.")).toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByText(/None yet/)).toBeInTheDocument();
  });

  it("will not save without a reason, and then sends only what changed with it", async () => {
    const calls = mockApi({
      "GET /admin/partner/status": () => [200, base],
      "PATCH /admin/partner": (b) => [200, { ...base, ...(b as object), connected: null }],
    });
    renderPage(<PartnerCard />);
    const user = userEvent.setup();
    const save = await screen.findByRole("button", { name: "Save partner settings" });
    expect(save).toBeDisabled();
    await user.click(screen.getByRole("switch", { name: "Use the partner" }));
    await user.click(screen.getByRole("switch", { name: "Feature price_history" }));
    const budget = screen.getByLabelText("Monthly call budget");
    await user.clear(budget);
    await user.type(budget, "2000");
    expect(save).toBeDisabled();
    expect(screen.getByText("Add a reason to save.")).toBeInTheDocument();
    await user.type(screen.getByLabelText(/Reason for the change/), "pause for the audit");
    expect(save).toBeEnabled();
    await user.click(save);
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true));
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({
      enabled: false,
      features: { price_history: false },
      monthly_call_budget: 2000,
      note: "pause for the audit",
    });
  });

  it("removes the cap with an empty budget and adds a named feature", async () => {
    const calls = mockApi({
      "GET /admin/partner/status": () => [200, base],
      "PATCH /admin/partner": () => [200, base],
    });
    renderPage(<PartnerCard />);
    const user = userEvent.setup();
    await user.clear(await screen.findByLabelText("Monthly call budget"));
    await user.type(screen.getByLabelText(/Add a feature/), "comparison");
    await user.type(screen.getByLabelText(/Reason for the change/), "new feature");
    await user.click(screen.getByRole("button", { name: "Save partner settings" }));
    await waitFor(() => expect(calls.some((c) => c.method === "PATCH")).toBe(true));
    expect(calls.find((c) => c.method === "PATCH")?.body).toEqual({
      features: { comparison: true },
      monthly_call_budget: null,
      note: "new feature",
    });
  });

  it("refuses a bad budget or feature name before sending anything", async () => {
    const calls = mockApi({ "GET /admin/partner/status": () => [200, base] });
    renderPage(<PartnerCard />);
    const user = userEvent.setup();
    const budget = await screen.findByLabelText("Monthly call budget");
    await user.clear(budget);
    await user.type(budget, "lots");
    await user.type(screen.getByLabelText(/Reason for the change/), "x");
    expect(screen.getByRole("button", { name: "Save partner settings" })).toBeDisabled();
    expect(budget).toHaveAttribute("aria-invalid", "true");
    await user.clear(budget);
    await user.type(budget, "5");
    await user.type(screen.getByLabelText(/Add a feature/), "Bad Name");
    expect(screen.getByRole("button", { name: "Save partner settings" })).toBeDisabled();
    expect(calls.filter((c) => c.method === "PATCH")).toHaveLength(0);
  });

  it("shows a failed load with a way to try again", async () => {
    mockApi({ "GET /admin/partner/status": () => [500, { error: { code: "internal", message: "Something went wrong" } }] });
    renderPage(<PartnerCard />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Something went wrong");
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("turns each fixed error code into plain words", () => {
    for (const code of ["unavailable", "rejected", "bad_response", "circuit_open", "budget_spent", "blocked", "anything_new"]) {
      const words = errorWords(code);
      expect(words).toBeTruthy();
      expect(words).not.toContain("_");
    }
    expect(errorWords(null)).toBeNull();
  });
});
