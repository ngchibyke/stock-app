import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./Button";
import { Icon, glyphs } from "./Icon";
import { ConfirmDialog } from "./Modal";
import { StatusBadge, Tabs, toneFor } from "./kit";

describe("Icon", () => {
  it("has a glyph with paths and a behaviour for every name", () => {
    for (const [name, g] of Object.entries(glyphs)) {
      expect(g.paths.length + (g.extra ? 1 : 0), name).toBeGreaterThan(0);
      expect(g.behaviour, name).toBeTruthy();
    }
  });

  it("is hidden from assistive tech unless it has a title", () => {
    const { container, rerender } = render(<Icon name="home" />);
    expect(container.querySelector("[aria-hidden='true']")).toBeInTheDocument();
    rerender(<Icon name="home" title="Home" />);
    expect(screen.getByRole("img", { name: "Home" })).toBeInTheDocument();
  });
});

describe("Button", () => {
  it("clicks", async () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Save</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("blocks clicks and says it is busy while loading", async () => {
    const onClick = vi.fn();
    render(
      <Button loading onClick={onClick}>
        Save
      </Button>,
    );
    const b = screen.getByRole("button", { name: /Save/ });
    expect(b).toBeDisabled();
    expect(b).toHaveAttribute("aria-busy", "true");
    await userEvent.click(b);
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe("badges and tabs", () => {
  it("colours statuses by meaning", () => {
    expect(toneFor("completed")).toBe("good");
    expect(toneFor("booking_failed")).toBe("bad");
    expect(toneFor("awaiting_seller")).toBe("warn");
    expect(toneFor("self")).toBe("neutral");
    render(<StatusBadge status="refund_pending" />);
    expect(screen.getByText("refund pending")).toBeInTheDocument();
  });

  it("selects a tab", async () => {
    function Harness() {
      const [v, setV] = useState<"a" | "b">("a");
      return <Tabs id="t" value={v} onChange={setV} tabs={[{ value: "a", label: "Open", count: 2 }, { value: "b", label: "Done" }]} />;
    }
    render(<Harness />);
    expect(screen.getByRole("tab", { name: /Open/ })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(screen.getByRole("tab", { name: "Done" }));
    expect(screen.getByRole("tab", { name: "Done" })).toHaveAttribute("aria-selected", "true");
  });
});

describe("ConfirmDialog", () => {
  it("will not confirm without a note, then passes the trimmed note", async () => {
    const onConfirm = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    render(<ConfirmDialog open title="Release" summary="Pay the seller" confirmLabel="Release now" onClose={onClose} onConfirm={onConfirm} />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Release now" }));
    expect(onConfirm).not.toHaveBeenCalled();
    await user.type(screen.getByPlaceholderText("Why are you doing this?"), "  checked with seller ");
    await user.click(screen.getByRole("button", { name: "Release now" }));
    expect(onConfirm).toHaveBeenCalledWith("checked with seller");
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("stays open when the action fails", async () => {
    const onConfirm = vi.fn().mockRejectedValue(new Error("no"));
    const onClose = vi.fn();
    render(<ConfirmDialog open title="Refund" summary="x" confirmLabel="Refund" danger onClose={onClose} onConfirm={onConfirm} />);
    const user = userEvent.setup();
    await user.type(screen.getByPlaceholderText("Why are you doing this?"), "why");
    await user.click(screen.getByRole("button", { name: "Refund" }));
    await vi.waitFor(() => expect(onConfirm).toHaveBeenCalled());
    expect(onClose).not.toHaveBeenCalled();
  });

  it("closes on Escape", async () => {
    const onClose = vi.fn();
    render(<ConfirmDialog open title="t" summary="s" confirmLabel="Go" noteLabel={false} onClose={onClose} onConfirm={async () => {}} />);
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });
});
