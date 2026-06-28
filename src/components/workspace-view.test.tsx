// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/monaco-local", () => ({}));
vi.mock("@monaco-editor/react", () => ({
  __esModule: true,
  default: () => null,
  loader: { config: () => {} },
}));

import { WorkspaceView } from "@/components/workspace-view";
import { type Connection, useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const connectedConnection: Connection = {
  id: "conn-1",
  name: "Cocoa Comaa",
  database: "reports",
  status: "Connected",
  engine: "MySQL",
  host: "localhost",
  port: 3306,
  user: "root",
  password: "",
  role: "",
  latency: "12 ms",
  ssl: true,
};

const postgresConnection: Connection = {
  ...connectedConnection,
  id: "conn-pg",
  name: "Postgres Dev",
  database: "postgres",
  engine: "PostgreSQL",
  port: 5432,
  user: "postgres",
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

    // Page-level tab strip.
    expect(screen.getByText("Overview")).toBeTruthy();
    expect(screen.getByText("Schema Map")).toBeTruthy();
    expect(screen.getByText("Query History")).toBeTruthy();
    expect(screen.getByText("Settings")).toBeTruthy();

    // Connection Details card
    expect(screen.getByText("Connection Details")).toBeTruthy();
    expect(screen.getByText("Host")).toBeTruthy();
    expect(screen.getAllByText("Database").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Engine").length).toBeGreaterThan(0);

    // Database Stats card with rendered totals.
    expect(screen.getByText("Database Stats")).toBeTruthy();
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

describe("WorkspaceView overview sub-tabs", () => {
  it("clicking a sub-tab swaps the body and updates the store", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);

    // Overview is the default body — the dashboard cards are visible.
    expect(screen.getByText("Recent Queries")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Tables" }));

    // Placeholder body for the Tables sub-tab is up; the cards are gone.
    expect(screen.queryByText("Recent Queries")).toBeNull();
    expect(screen.getAllByText("Tables").length).toBeGreaterThan(0);
    expect(useAppStore.getState().connectionOverviewTab["conn-1"]).toBe(
      "tables",
    );
  });

  it("Schemas sub-tab shows the Postgres-only panel on non-PG engines", () => {
    const mysqlConnection: Connection = {
      id: "conn-mysql",
      name: "MySQL Reports",
      database: "reports",
      status: "Connected",
      engine: "MySQL",
      host: "localhost",
      port: 3306,
      user: "root",
      password: "",
      role: "",
      latency: "8 ms",
      ssl: false,
    };

    useAppStore.setState({
      activeConnectionId: "conn-mysql",
      activeTabId: "",
      connections: [mysqlConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-mysql": [{ name: "reports", tables: ["events"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Schemas" }));

    expect(screen.getByText(/Schemas is Postgres-only/)).toBeTruthy();
  });

  it("Schemas sub-tab map action switches to the Schema Map sub-tab", () => {
    useAppStore.setState({
      activeConnectionId: "conn-pg",
      activeTabId: "",
      connections: [postgresConnection],
      workspaceTabs: [],
      connectionOverviewTab: { "conn-pg": "schemas" },
      schemaExplorer: {
        "conn-pg": [
          { name: "public", tables: ["users"], views: [] },
          { name: "audit", tables: ["events"], views: [] },
        ],
      },
      relationStats: {
        "conn-pg": [
          {
            schema: "audit",
            name: "events",
            kind: "table",
            rowCountEstimate: 10,
            totalSizeBytes: 1024,
          },
        ],
      },
      relationStatsStatus: { "conn-pg": { state: "success" } },
    });

    render(<WorkspaceView isClient={false} />);

    fireEvent.click(screen.getByLabelText("View audit schema map"));

    expect(useAppStore.getState().connectionOverviewTab["conn-pg"]).toBe(
      "schema-map",
    );
    expect(useAppStore.getState().connectionSchemaMapSchema["conn-pg"]).toBe(
      "audit",
    );
  });

  it("the Recent Queries 'View all' button switches to the Query History sub-tab", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
      },
    });

    render(<WorkspaceView isClient={false} />);

    fireEvent.click(screen.getByRole("button", { name: "View all" }));

    expect(useAppStore.getState().connectionOverviewTab["conn-1"]).toBe(
      "query-history",
    );
    // The real Query History body has rendered (matches the card title).
    expect(screen.getByText("Query history")).toBeTruthy();
  });
});
