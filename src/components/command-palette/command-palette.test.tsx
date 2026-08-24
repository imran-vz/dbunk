/* oxlint-disable anti-slop/no-chained-type-assertions, anti-slop/require-safety-comment-for-type-assertion, anti-slop/no-module-mocking -- jsdom capability shims install minimal fakes at the global boundary; sonner is mocked to observe refusal toasts. */
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

const toastMocks = vi.hoisted(() => ({ error: vi.fn(), success: vi.fn() }));
vi.mock("sonner", () => ({ toast: toastMocks }));

import { CommandPalette } from "@/components/command-palette/command-palette";
import { useShortcutHandler } from "@/lib/shortcuts";
import { type Connection, useAppStore } from "@/lib/store";
import { uiSet } from "@/lib/ui-state";

const initialStoreState = useAppStore.getState();

function Harness({ onToggle }: { onToggle: () => void }) {
  useShortcutHandler("toggle-navigator", onToggle);
  return <CommandPalette />;
}

const connection = (
  id: string,
  name: string,
  status: Connection["status"],
): Connection => ({
  id,
  name,
  database: "app",
  host: "h",
  port: 5432,
  user: "u",
  password: "",
  role: "",
  engine: "PostgreSQL",
  ssl: false,
  status,
  latency: "1ms",
});

/** Two connections (one disconnected), mixed relation kinds, a saved
 *  query, and a history entry — the standard Open Anything fixture. */
const seedStore = () => {
  useAppStore.setState({
    activeConnectionId: "c1",
    connections: [
      connection("c1", "Local", "Connected"),
      connection("c2", "Analytics", "Disconnected"),
    ],
    schemaExplorer: {
      c1: [
        {
          name: "public",
          tables: ["orders", "orders_archive"],
          views: ["active_orders"],
          materializedViews: ["orders_daily"],
        },
      ],
    },
    savedQueries: [
      {
        id: "sq1",
        name: "Orders by day",
        body: "SELECT 1",
        connectionId: "c1",
        isFavorite: false,
        ownerId: null,
        createdAt: "2026-08-24T00:00:00Z",
        updatedAt: "2026-08-24T00:00:00Z",
      },
      {
        id: "sq2",
        name: "Orphaned query",
        body: "SELECT 2",
        connectionId: "conn-gone",
        isFavorite: false,
        ownerId: null,
        createdAt: "2026-08-24T00:00:00Z",
        updatedAt: "2026-08-24T00:00:00Z",
      },
    ],
    queryHistory: [
      {
        id: "h1",
        sql: "SELECT * FROM orders LIMIT 10",
        connectionId: "c1",
        connectionName: "Local",
        database: "app",
        engine: "PostgreSQL",
        status: "success",
        runtimeMs: 12,
        startedAt: "2026-08-24T00:00:00Z",
      },
    ],
  });
};

beforeEach(() => {
  window.localStorage.clear();
  toastMocks.error.mockClear();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

const openPalette = () =>
  fireEvent.keyDown(window, { key: "k", metaKey: true });

const paletteInput = () => screen.getByPlaceholderText(/Open anything/);

const type = (value: string) =>
  fireEvent.change(paletteInput(), { target: { value } });

const itemRows = () => Array.from(document.querySelectorAll('[cmdk-item=""]'));

describe("Open Anything palette (Plan 010, mock A)", () => {
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
    seedStore();
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("> ");
    const kinds = itemRows().map((row) => row.textContent ?? "");
    expect(kinds.length).toBeGreaterThan(0);
    expect(kinds.every((text) => text.includes("Cmd"))).toBe(true);
    expect(screen.queryByText("Local")).toBeNull();
  });

  it("renders a flat ranked list where the exact match is first and Enter runs it", () => {
    seedStore();
    const openTableTab = vi.fn();
    useAppStore.setState({ openTableTab });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("orders");
    const rows = itemRows();
    // Exact label match first, regardless of kind; weaker matches after.
    expect(rows[0]?.textContent).toContain("orders");
    expect(rows[0]?.textContent).toContain("Table");
    expect(rows[0]?.getAttribute("aria-selected")).toBe("true");

    fireEvent.keyDown(paletteInput(), { key: "Enter" });
    expect(openTableTab).toHaveBeenCalledWith("public", "orders");
  });

  it("opens views and materialized views through openViewTab, not table browse", () => {
    seedStore();
    const openTableTab = vi.fn();
    const openViewTab = vi.fn();
    useAppStore.setState({ openTableTab, openViewTab });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("active_orders");
    fireEvent.click(screen.getByText("active_orders"));
    expect(openViewTab).toHaveBeenCalledWith("public", "active_orders");
    expect(openTableTab).not.toHaveBeenCalled();
  });

  it("switches the active connection before opening a cross-connection relation", () => {
    seedStore();
    useAppStore.setState({
      activeConnectionId: "c2",
      connections: [
        connection("c1", "Local", "Connected"),
        connection("c2", "Analytics", "Connected"),
      ],
    });
    const openTableTab = vi.fn(() => {
      expect(useAppStore.getState().activeConnectionId).toBe("c1");
    });
    useAppStore.setState({ openTableTab });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("orders_archive");
    fireEvent.click(screen.getByText("orders_archive"));
    expect(openTableTab).toHaveBeenCalledWith("public", "orders_archive");
  });

  it("connects (and focuses) a disconnected connection row", () => {
    seedStore();
    const connectConnection = vi.fn(async () => {});
    useAppStore.setState({ connectConnection });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("Analytics");
    fireEvent.click(screen.getByText("Analytics"));
    expect(connectConnection).toHaveBeenCalledWith("c2");
    expect(useAppStore.getState().activeConnectionId).toBe("c2");
  });

  it("reveals a schema: expands the node, switches connection, signals the rail", () => {
    seedStore();
    useAppStore.setState({ activeConnectionId: "c2" });
    const before = useAppStore.getState().railRevealRequest;
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("public");
    fireEvent.click(screen.getByText("public"));
    const state = useAppStore.getState();
    expect(state.expandedSchemas).toContain("c1:public");
    expect(state.activeConnectionId).toBe("c1");
    expect(state.railRevealRequest).toBe(before + 1);
  });

  it("refuses a saved query with no resolvable connection via toast, never a broken tab", () => {
    seedStore();
    useAppStore.setState({ activeConnectionId: "" });
    const openWorkspaceTab = vi.fn();
    useAppStore.setState({ openWorkspaceTab });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("Orphaned");
    fireEvent.click(screen.getByText("Orphaned query"));
    expect(toastMocks.error).toHaveBeenCalledTimes(1);
    expect(openWorkspaceTab).not.toHaveBeenCalled();
  });

  it("discloses truncated matches in the footer instead of cutting silently", () => {
    seedStore();
    useAppStore.setState({
      schemaExplorer: {
        c1: [
          {
            name: "public",
            tables: Array.from({ length: 250 }, (_, i) => `noise_${i}`),
          },
        ],
      },
    });
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    type("noise");
    const footer = screen.getByTestId("palette-truncation");
    expect(footer.textContent).toContain("50 more matches");
    expect(itemRows().length).toBe(200);
  });

  it("migrates pre-Open-Anything table frecency keys onto relation keys", () => {
    seedStore();
    uiSet(
      "dbunk.palette.frecency",
      JSON.stringify({
        "table:c1::public::orders_archive": { count: 5, last: Date.now() },
      }),
    );
    render(<Harness onToggle={vi.fn()} />);
    openPalette();

    // Empty query: recents first — the migrated table leads.
    expect(itemRows()[0]?.textContent).toContain("orders_archive");
  });

  it("records frecency and ranks the used item first on reopen", () => {
    const onToggle = vi.fn();
    render(<Harness onToggle={onToggle} />);
    openPalette();
    fireEvent.click(screen.getByText("Toggle navigator"));

    openPalette();
    expect(itemRows()[0]?.textContent).toContain("Toggle navigator");
  });
});
