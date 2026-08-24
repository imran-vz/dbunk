/* oxlint-disable anti-slop/no-chained-type-assertions, anti-slop/require-safety-comment-for-type-assertion -- jsdom capability shims install minimal fakes at the global boundary. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// cmdk observes its list size; jsdom has no ResizeObserver.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
// oxlint-disable-next-line anti-slop/no-runtime-typeof -- jsdom capability shim.
if (typeof globalThis.ResizeObserver === "undefined") {
  globalThis.ResizeObserver =
    ResizeObserverStub as unknown as typeof ResizeObserver;
}
// cmdk scrolls the selected row into view; jsdom lacks scrollIntoView.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {};
}

import { CommandPalette } from "@/components/command-palette/command-palette";
import { useShortcutHandler } from "@/lib/shortcuts";
import { useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

function Harness({ onToggle }: { onToggle: () => void }) {
  useShortcutHandler("toggle-navigator", onToggle);
  return <CommandPalette />;
}

beforeEach(() => {
  window.localStorage.clear();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

const openPalette = () =>
  fireEvent.keyDown(window, { key: "k", metaKey: true });

describe("CommandPalette (§4.10)", () => {
  it("lists registered commands with their kbd hint and runs them", () => {
    const onToggle = vi.fn();
    render(<Harness onToggle={onToggle} />);
    openPalette();

    const row = screen.getByText("Toggle navigator");
    // Hint comes from the shortcut registry (mod+B → Ctrl B off-mac).
    expect(row.closest('[cmdk-item=""]')?.textContent).toContain("B");

    fireEvent.click(row);
    expect(onToggle).toHaveBeenCalledTimes(1);
    // Palette closes after running.
    expect(screen.queryByText("Toggle navigator")).toBeNull();
  });

  it("restricts to commands with the > prefix", () => {
    useAppStore.setState({
      connections: [
        {
          id: "c1",
          name: "Local",
          database: "app",
          host: "h",
          port: 5432,
          user: "u",
          password: "",
          role: "",
          engine: "PostgreSQL",
          ssl: false,
          status: "Connected",
          latency: "1ms",
        },
      ],
    });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();
    expect(screen.getByText("Connections")).toBeTruthy();

    const input = screen.getByPlaceholderText(/Search commands/);
    fireEvent.change(input, { target: { value: ">" } });
    expect(screen.queryByText("Connections")).toBeNull();
    expect(screen.getByText("Commands")).toBeTruthy();
  });

  it("records frecency and surfaces recents on reopen", () => {
    const onToggle = vi.fn();
    render(<Harness onToggle={onToggle} />);
    openPalette();
    fireEvent.click(screen.getByText("Toggle navigator"));

    openPalette();
    expect(screen.getByText("Recent")).toBeTruthy();
  });
});
