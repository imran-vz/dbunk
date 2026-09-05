import { cleanup, render, screen, waitFor } from "@testing-library/react";
/* oxlint-disable anti-slop/no-module-mocking -- This harness isolates workbench routing from unrelated editor surfaces. */
import type { ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/components/pg-tool-jobs/workspace", () => ({
  PgToolWorkspace: ({
    connection,
    table,
  }: {
    connection: { id: string };
    table?: { schema: string; name: string };
  }) => (
    <output data-testid="pg-tool-target">
      {connection.id}:{table?.schema ?? "database"}:{table?.name ?? "database"}
    </output>
  ),
}));
vi.mock("@/components/ui/panel", () => ({
  Panel: ({ children }: { children: ReactNode }) => <aside>{children}</aside>,
  useLayoutPressure: () => undefined,
  usePanelState: () => ({
    expand: vi.fn(),
    setSize: vi.fn(),
    toggle: vi.fn(),
  }),
}));
vi.mock("@/components/workbench/database-navigator", () => ({
  DatabaseNavigator: () => <nav data-testid="database-navigator" />,
}));
vi.mock("@/components/workbench/object-tab-row", () => ({
  ObjectTabRow: () => <div data-testid="object-tabs" />,
  TableSectionToggle: () => null,
}));
vi.mock("@/components/workbench/workbench-shell", () => ({
  WorkbenchShell: ({
    activeRail,
    railItems,
    children,
  }: {
    activeRail: string;
    railItems: ReadonlyArray<{ id: string }>;
    children: ReactNode;
  }) => (
    <main
      data-testid="workbench-shell"
      data-active-rail={activeRail}
      data-rail-items={railItems.map((item) => item.id).join(",")}
    >
      {children}
    </main>
  ),
}));
vi.mock("@/lib/shortcuts", () => ({
  useShortcutHandler: () => undefined,
}));

import { type Connection, useAppStore } from "@/lib/store";

import { RelationalWorkbench } from "./relational-workbench";

const postgres = (id: string): Connection => ({
  id,
  name: id,
  database: `${id}-database`,
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "4 ms",
  ssl: false,
});
const mysql: Connection = {
  id: "mysql",
  name: "MySQL",
  database: "app",
  status: "Connected",
  engine: "MySQL",
  host: "localhost",
  port: 3306,
  user: "root",
  password: "",
  role: "admin",
  latency: "3 ms",
  ssl: false,
};
const initialStore = useAppStore.getState();

function renderWorkbench() {
  return render(
    <RelationalWorkbench
      isClient
      isWindowFullscreen={false}
      onPointerDown={() => undefined}
      onDoubleClick={() => undefined}
    />,
  );
}

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState(initialStore, true);
});

afterEach(() => {
  cleanup();
  localStorage.clear();
  useAppStore.setState(initialStore, true);
  vi.restoreAllMocks();
});

describe("relational workbench PostgreSQL tool routing", () => {
  it("uses the contextual tab connection after the active connection changes", () => {
    const contextualConnection = postgres("contextual");
    const activeConnection = postgres("active");
    useAppStore.setState({
      connections: [contextualConnection, activeConnection],
      activeConnectionId: activeConnection.id,
      activeTabId: "tool-tab",
      workspaceTabs: [
        {
          id: "tool-tab",
          kind: "pg-tools",
          label: "Backup users",
          connectionId: contextualConnection.id,
          schema: "Sales Ops",
          table: "Order.Items",
          toolOperation: "backup",
        },
      ],
    });

    renderWorkbench();

    expect(screen.getByTestId("pg-tool-target").textContent).toBe(
      "contextual:Sales Ops:Order.Items",
    );
  });

  it("normalizes a persisted PostgreSQL-only rail for another engine", async () => {
    localStorage.setItem("dbunk.workbench.rail", "pg-tools");
    useAppStore.setState({
      connections: [mysql],
      activeConnectionId: mysql.id,
      activeTabId: "",
      workspaceTabs: [],
    });

    renderWorkbench();

    await waitFor(() =>
      expect(screen.getByTestId("workbench-shell").dataset.activeRail).toBe(
        "tables",
      ),
    );
    expect(
      screen.getByTestId("workbench-shell").dataset.railItems,
    ).not.toContain("pg-tools");
    expect(localStorage.getItem("dbunk.workbench.rail")).toBe("tables");
  });
});
