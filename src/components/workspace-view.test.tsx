// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
}));

vi.mock("@/components/schema-relationship-map", () => ({
  SchemaRelationshipMap: ({
    schema,
    connectionId,
  }: {
    schema: string;
    connectionId: string;
  }) => (
    <div data-testid="workspace-schema-map">
      {connectionId}:{schema}
    </div>
  ),
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
  it("shows connected database stats, schemas, tables, and schema map when no tab is open", () => {
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
        },
      },
      databaseOverviewStatsStatus: {
        "conn-1": { state: "success" },
      },
    });

    render(<WorkspaceView isClient={false} />);

    expect(screen.getByText("Cocoa Comaa")).toBeTruthy();
    expect(screen.getAllByText("Schemas").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Tables").length).toBeGreaterThan(0);
    expect(screen.getByText("Views")).toBeTruthy();
    expect(screen.getByText("Database size")).toBeTruthy();
    expect(screen.getByText("10 MB")).toBeTruthy();
    expect(screen.getByText("Table size")).toBeTruthy();
    expect(screen.getByText("4 MB")).toBeTruthy();
    expect(screen.getByText("Index size")).toBeTruthy();
    expect(screen.getByText("2 MB")).toBeTruthy();
    expect(screen.getByText("public")).toBeTruthy();
    expect(screen.getByText("audit")).toBeTruthy();
    expect(screen.getByText("users")).toBeTruthy();
    expect(screen.getByText("orders")).toBeTruthy();
    expect(screen.getByText("active_users")).toBeTruthy();
    expect(screen.getByTestId("workspace-schema-map").textContent).toBe(
      "conn-1:public",
    );
  });

  it("switches the schema map when a schema is selected", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      activeTabId: "",
      connections: [connectedConnection],
      workspaceTabs: [],
      schemaExplorer: {
        "conn-1": [
          { name: "public", tables: ["users"], views: [] },
          { name: "audit", tables: ["events"], views: [] },
        ],
      },
    });

    render(<WorkspaceView isClient={false} />);

    fireEvent.click(
      screen.getByRole("button", { name: /select schema audit/i }),
    );

    expect(screen.getByTestId("workspace-schema-map").textContent).toBe(
      "conn-1:audit",
    );
  });

  it("loads overview schemas from the displayed fallback connection", () => {
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

    expect(screen.getByText("public")).toBeTruthy();
    expect(screen.getByText("users")).toBeTruthy();
    expect(screen.getByTestId("workspace-schema-map").textContent).toBe(
      "conn-1:public",
    );
  });

  it("opens a table from the overview table list", () => {
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

    fireEvent.click(
      screen.getByRole("button", { name: /open table public\.users/i }),
    );

    expect(openTableTab).toHaveBeenCalledWith("public", "users");
  });
});
