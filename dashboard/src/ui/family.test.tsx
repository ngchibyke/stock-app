import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";
import { useTheme } from "../theme";
import { CommandPalette } from "./CommandPalette";
import { Banner, Checkbox } from "./Controls";
import { Menu } from "./Menu";
import { Modal } from "./Modal";
import { PageHeader } from "./PageHeader";
import { Skeleton, SkeletonShell, SkeletonTable } from "./Skeleton";
import { Tooltip } from "./Tooltip";

describe("Skeleton", () => {
  it("announces itself once and keeps its blocks out of the reading order", () => {
    const { container } = render(<SkeletonTable rows={3} cols={4} label="Loading orders" />);
    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("aria-busy", "true");
    expect(within(status).getByText("Loading orders")).toBeInTheDocument();
    expect(container.querySelectorAll(".skel-row")).toHaveLength(4); // a header strip and three rows
    expect(container.querySelectorAll(".bone[aria-hidden='true']").length).toBeGreaterThan(8);
  });

  it("takes a line count", () => {
    const { container } = render(<Skeleton lines={5} />);
    expect(container.querySelectorAll(".bone")).toHaveLength(5);
  });

  it("stands in for the whole app frame", () => {
    render(<SkeletonShell />);
    expect(screen.getByText("Loading the dashboard")).toBeInTheDocument();
  });
});

describe("PageHeader", () => {
  it("names the section from the route and shows its actions", () => {
    render(
      <MemoryRouter initialEntries={["/orders"]}>
        <PageHeader title="Orders" subtitle="Every order." actions={<button type="button">Export</button>} />
      </MemoryRouter>,
    );
    expect(screen.getByRole("heading", { level: 1, name: "Orders" })).toBeInTheDocument();
    expect(screen.getByText("Money")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Export" })).toBeInTheDocument();
  });
});

describe("Tooltip", () => {
  it("opens on keyboard focus, describes its child, and closes on Esc", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Copy the id">
        <button type="button">Copy</button>
      </Tooltip>,
    );
    await user.tab();
    const tip = await screen.findByRole("tooltip");
    expect(tip).toHaveTextContent("Copy the id");
    expect(screen.getByRole("button", { name: "Copy" })).toHaveAttribute("aria-describedby", tip.id);
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
  });

  it("waits a moment before opening on hover, and closes when the pointer leaves", async () => {
    const user = userEvent.setup();
    render(
      <Tooltip label="Wait for it" delay={120}>
        <button type="button">Hover</button>
      </Tooltip>,
    );
    await user.hover(screen.getByRole("button", { name: "Hover" }));
    expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
    expect(await screen.findByRole("tooltip")).toHaveTextContent("Wait for it");
    await user.unhover(screen.getByRole("button", { name: "Hover" }));
    await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
  });
});

describe("Menu", () => {
  const items = (spy: (s: string) => void) => [
    { label: "Open", onSelect: () => spy("open") },
    { label: "Copy ID", onSelect: () => spy("copy") },
    "separator" as const,
    { label: "Locked", onSelect: () => spy("locked"), disabled: true },
    { label: "Suspend", onSelect: () => spy("suspend"), danger: true },
  ];

  it("opens from the button, moves with the arrows past a disabled item, and picks with Enter", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Menu label="Row actions" items={items(spy)} />);
    await user.click(screen.getByRole("button", { name: "Row actions" }));
    const menu = await screen.findByRole("menu", { name: "Row actions" });
    expect(within(menu).getAllByRole("menuitem")).toHaveLength(4);
    await waitFor(() => expect(menu).toHaveFocus());
    await user.keyboard("{ArrowDown}{ArrowDown}"); // Open -> Copy ID -> (Locked is skipped) Suspend
    await user.keyboard("{Enter}");
    expect(spy).toHaveBeenCalledWith("suspend");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Row actions" })).toHaveFocus();
  });

  it("jumps by typing a letter, and Esc closes without choosing", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Menu label="Row actions" items={items(spy)} />);
    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await screen.findByRole("menu");
    await user.keyboard("s");
    expect(screen.getByRole("menuitem", { name: "Suspend" })).toHaveClass("active");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
    expect(spy).not.toHaveBeenCalled();
  });

  it("does not run a disabled item", async () => {
    const user = userEvent.setup();
    const spy = vi.fn();
    render(<Menu label="Row actions" items={items(spy)} />);
    await user.click(screen.getByRole("button", { name: "Row actions" }));
    await user.click(await screen.findByRole("menuitem", { name: "Locked" }));
    expect(spy).not.toHaveBeenCalled();
  });
});

describe("CommandPalette", () => {
  const commands = (run: (s: string) => void) => [
    { id: "orders", label: "Orders", group: "Go to · Money", icon: "orders" as const, run: () => run("orders") },
    { id: "audit", label: "Audit log", group: "Go to · Configure", icon: "audit" as const, run: () => run("audit") },
    { id: "theme", label: "Switch light or dark theme", group: "Actions", icon: "sparkle" as const, keywords: "appearance", run: () => run("theme") },
  ];

  it("narrows as you type, finds by keyword, and runs the chosen command", async () => {
    const user = userEvent.setup();
    const run = vi.fn();
    const onClose = vi.fn();
    render(<CommandPalette open onClose={onClose} commands={commands(run)} />);
    expect(screen.getAllByRole("option")).toHaveLength(3);
    await user.type(screen.getByRole("combobox"), "appearance");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    await user.keyboard("{Enter}");
    expect(run).toHaveBeenCalledWith("theme");
    expect(onClose).toHaveBeenCalled();
  });

  it("moves with the arrows", async () => {
    const user = userEvent.setup();
    const run = vi.fn();
    render(<CommandPalette open onClose={() => {}} commands={commands(run)} />);
    await waitFor(() => expect(screen.getByRole("combobox")).toHaveFocus());
    await user.keyboard("{ArrowDown}{Enter}");
    expect(run).toHaveBeenCalledWith("audit");
  });

  it("shows an empty message", async () => {
    const user = userEvent.setup();
    render(<CommandPalette open onClose={() => {}} commands={commands(() => {})} />);
    await user.type(screen.getByRole("combobox"), "zzz");
    expect(screen.getByText(/Nothing matches/)).toBeInTheDocument();
  });
});

describe("Modal", () => {
  function Host({ onClose }: { onClose: () => void }) {
    return (
      <>
        <button type="button">Opener</button>
        <Modal open title="Rename" description="Pick a clear name." onClose={onClose} footer={<button type="button">Save</button>}>
          <input aria-label="Name" />
        </Modal>
      </>
    );
  }

  it("is described by its description, focuses the first field, keeps Tab inside, and Esc closes", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Host onClose={onClose} />);
    const dialog = await screen.findByRole("dialog", { name: "Rename" });
    expect(dialog).toHaveAccessibleDescription("Pick a clear name.");
    await waitFor(() => expect(screen.getByLabelText("Name")).toHaveFocus());
    for (let i = 0; i < 6; i++) {
      await user.tab();
      expect(dialog.contains(document.activeElement)).toBe(true);
    }
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("locks page scroll while open and gives it back", () => {
    const { rerender } = render(<Modal open title="A" onClose={() => {}}>x</Modal>);
    expect(document.body.style.overflow).toBe("hidden");
    rerender(<Modal open={false} title="A" onClose={() => {}}>x</Modal>);
    expect(document.body.style.overflow).not.toBe("hidden");
  });
});

describe("Controls", () => {
  it("toggles a checkbox from its label", async () => {
    const user = userEvent.setup();
    function Host() {
      const [on, setOn] = useState(false);
      return <Checkbox checked={on} onChange={setOn} label="I checked" hint="Kept in the audit log." />;
    }
    render(<Host />);
    const box = screen.getByRole("checkbox", { name: /I checked/ });
    expect(box).not.toBeChecked();
    await user.click(screen.getByText("I checked"));
    expect(box).toBeChecked();
  });

  it("speaks a warning as an alert and a note as a status", () => {
    const { rerender } = render(<Banner tone="warn" title="Careful">Read this.</Banner>);
    expect(screen.getByRole("alert")).toHaveTextContent("Careful");
    rerender(<Banner tone="info">FYI</Banner>);
    expect(screen.getByRole("status")).toHaveTextContent("FYI");
  });
});

describe("theme", () => {
  it("is one theme for every switch on the page", async () => {
    function Switch({ id }: { id: string }) {
      const [theme, toggle] = useTheme();
      return (
        <button type="button" onClick={toggle}>
          {id}:{theme}
        </button>
      );
    }
    const user = userEvent.setup();
    render(
      <>
        <Switch id="a" />
        <Switch id="b" />
      </>,
    );
    const before = screen.getByRole("button", { name: /^a:/ }).textContent?.split(":")[1];
    await user.click(screen.getByRole("button", { name: /^a:/ }));
    const after = before === "dark" ? "light" : "dark";
    expect(screen.getByRole("button", { name: `a:${after}` })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: `b:${after}` })).toBeInTheDocument();
    expect(document.documentElement.dataset.theme).toBe(after);
  });
});
