import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { mockApi, renderPage } from "../test/helpers";
import { EmptyState } from "../ui/kit";
import { Codes } from "./Codes";
import { Countdown, Disputes, waitTone } from "./Disputes";
import { Orders } from "./Orders";

afterEach(() => vi.restoreAllMocks());

const HOUR = 3_600_000;

describe("how worried to be about a dispute", () => {
  it("is calm under a day, amber from a day, red from three", () => {
    expect(waitTone(2 * HOUR)).toBe("calm");
    expect(waitTone(24 * HOUR)).toBe("warn");
    expect(waitTone(71 * HOUR)).toBe("warn");
    expect(waitTone(72 * HOUR)).toBe("bad");
  });

  it("shows the seller's clock amber under a day and red under six hours", () => {
    const now = Date.parse("2026-11-19T10:00:00Z");
    const at = (h: number) => new Date(now + h * HOUR).toISOString();
    const { container, rerender } = render(<Countdown until={at(30)} now={now} />);
    expect(container.querySelector(".countdown")).not.toHaveClass("soon", "urgent");
    rerender(<Countdown until={at(20)} now={now} />);
    expect(container.querySelector(".countdown")).toHaveClass("soon");
    rerender(<Countdown until={at(5)} now={now} />);
    expect(container.querySelector(".countdown")).toHaveClass("urgent");
  });

  it("marks a long-waiting dispute in the queue", async () => {
    const opened = (h: number) => new Date(Date.now() - h * HOUR).toISOString();
    const row = (id: string, h: number) => ({ id, order_id: `${id}aaaaaaaa`, status: "awaiting_decision", reason: "damaged", opened_at: opened(h), seller_respond_by: opened(1), items_total_kobo: 1_000_000 });
    mockApi({ "GET /admin/disputes": () => [200, { items: [row("d1", 2), row("d2", 30), row("d3", 80)] }] });
    const { container } = renderPage(<Disputes />);
    await screen.findByText("2 hr ago");
    expect([...container.querySelectorAll(".age")].map((a) => a.className.replace("age ", ""))).toEqual(["age-calm", "age-warn", "age-bad"]);
  });
});

describe("empty states with a way forward", () => {
  it("shows an action under the hint and runs it", async () => {
    const onClick = vi.fn();
    render(<EmptyState icon="inbox" title="Nothing here" hint="Try again." action={<button type="button" onClick={onClick}>Do it</button>} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Do it" }));
    expect(onClick).toHaveBeenCalled();
  });

  it("offers to create the first access code, and opens the dialog", async () => {
    mockApi({ "GET /admin/access-codes": () => [200, []] });
    renderPage(<Codes />);
    await userEvent.setup().click(await screen.findByRole("button", { name: "Create the first code" }));
    expect(await screen.findByRole("dialog", { name: "New access code" })).toBeInTheDocument();
  });

  it("offers to clear the filters when they hide every order, and clears them", async () => {
    const user = userEvent.setup();
    const calls = mockApi({ "GET /admin/orders": () => [200, { items: [], next_cursor: null }] });
    renderPage(<Orders />);
    // Nothing is filtered yet, so there is nothing to clear.
    await screen.findByText("No orders match");
    expect(screen.queryByRole("button", { name: "Clear filters" })).not.toBeInTheDocument();
    await user.selectOptions(screen.getByRole("combobox"), "paid");
    const clear = await screen.findByRole("button", { name: "Clear filters" });
    await user.click(clear);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Clear filters" })).not.toBeInTheDocument());
    expect(calls.at(-1)?.query.get("status") ?? "").toBe("");
  });
});
