// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { WorkspaceView } from "@/components/workspace-view";
import { type Connection, useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const connectedConnection: Connection = {
  id: "conn-1",
  name: "Cocoa Comaa",
  database: "postgres",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  latency: "12 ms",
  lastSync: "Just now",
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
  vi.clearAllMocks();
});

describe("WorkspaceView database overview", () => {
  it("renders the connection header, page tabs, and metric cards", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [
          {
            name: "public",
            tables: ["users", "orders"],
            views: ["active_users"],
          },
          { name: "audit", tables: ["events"], views: [] },
        ],
      },
      databaseOverviewStats: {
        "conn-1": {
          databaseSizeBytes: 10485760,
          tableSizeBytes: 4194304,
          indexSizeBytes: 2097152,
          tableCount: 3,
          schemaCount: 2,
          rowCountEstimate: 1234567,
          indexCount: 12,
          connectionCount: 4,
        },
      },
      databaseOverviewStatsStatus: {
        "conn-1": { state: "success" },
      },
    });

    render(<WorkspaceView isClient={false} />);

    // Header
    expect(screen.getByText("Cocoa Comaa")).toBeTruthy();
    expect(screen.getAllByText("Connected").length).toBeGreaterThan(0);

    // Page-level tab strip (visual-only, all six render)
    expect(screen.getByText("Overview")).toBeTruthy();
    expect(screen.getByText("Query History")).toBeTruthy();
    expect(screen.getByText("Settings")).toBeTruthy();

    // Connection Details card
    expect(screen.getByText("Connection Details")).toBeTruthy();
    expect(screen.getByText("Host")).toBeTruthy();
    expect(screen.getAllByText("Database").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Engine").length).toBeGreaterThan(0);

    // Database Stats card with rendered totals. The default fixture is
    // PostgreSQL, so the row-count label shows the (≈) suffix to flag
    // that pg_class.reltuples is an estimate.
    expect(screen.getByText("Database Stats")).toBeTruthy();
    expect(screen.getByText("Rows (≈)")).toBeTruthy();
    expect(screen.getByText("Indexes")).toBeTruthy();

    // Recent Queries + Favorite Tables sections + health banner
    expect(screen.getByText("Recent Queries")).toBeTruthy();
    expect(screen.getByText("Favorite Tables")).toBeTruthy();
    expect(screen.getByText("Your connection is healthy")).toBeTruthy();
  });

  it("hides missing round-trip latency in the health banner", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [{ ...connectedConnection, latency: "undefined ms" }],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);

    expect(screen.getByText("Your connection is healthy")).toBeTruthy();
    expect(screen.queryByText(/Round-trip/i)).toBeNull();
    expect(screen.getByText(/Last checked/i)).toBeTruthy();
  });

  it("opens a table from the favorite tables card", () => {
    const openTableTab = vi.fn();
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      openTableTab,
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);

    // Card lists "users" with schema "public" — click the row.
    const usersRow = screen.getByText("users");
    fireEvent.click(usersRow.closest("button") ?? usersRow);

    expect(openTableTab).toHaveBeenCalledWith("public", "users");
  });

  it("falls back to the first connection when none is active", () => {
    useAppStore.setState({
      activeConnectionId: "",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);

    expect(screen.getByText("Cocoa Comaa")).toBeTruthy();
    expect(screen.getByText("users")).toBeTruthy();
  });
});
