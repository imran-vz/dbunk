// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { OverviewRailView } from "@/components/workspace-overview/overview-rail-view";
import { type Connection, useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const connection: Connection = {
  id: "conn-1",
  name: "Local Postgres",
  database: "app",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  engine: "PostgreSQL",
  ssl: false,
  status: "Connected",
  latency: "2ms",
};

const schemas = [
  { name: "public", tables: ["users", "orders"], views: [] },
  { name: "audit", tables: ["events"], views: [] },
];

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    queryHistory: [
      {
        id: "q1",
        sql: "select *\nfrom users;",
        connectionId: "conn-1",
        connectionName: "Local Postgres",
        database: "app",
        engine: "PostgreSQL",
        status: "success",
        runtimeMs: 12,
        rowCount: 3,
        startedAt: "2026-08-20T10:00:00Z",
      },
      {
        id: "q2",
        sql: "select 1;",
        connectionId: "other-conn",
        connectionName: "Other",
        database: "x",
        engine: "PostgreSQL",
        status: "success",
        runtimeMs: 5,
        rowCount: 1,
        startedAt: "2026-08-20T10:00:00Z",
      },
    ],
  });
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("OverviewRailView (§5.7)", () => {
  it("renders health, stats, the table catalog, and recent queries as dense rows", () => {
    render(
      <OverviewRailView
        activeConnection={connection}
        schemas={schemas}
        isConnected
        onOpenTable={vi.fn()}
        onReopenQuery={vi.fn()}
      />,
    );

    expect(screen.getByText("Local Postgres")).toBeTruthy();
    expect(screen.getByText("Connected · 2ms")).toBeTruthy();
    // Stats fall back to local schema counts before server stats land.
    expect(screen.getByText("Tables")).toBeTruthy();
    expect(screen.getByText("Tables · 3")).toBeTruthy();
    expect(
      screen.getByRole("button", { name: /public\.\s*users/ }),
    ).toBeTruthy();
    // Only this connection's history shows, flattened to one line.
    expect(screen.getByText("select * from users;")).toBeTruthy();
    expect(screen.queryByText("select 1;")).toBeNull();
  });

  it("opens tables and reopens queries from the rows", () => {
    const onOpenTable = vi.fn();
    const onReopenQuery = vi.fn();
    render(
      <OverviewRailView
        activeConnection={connection}
        schemas={schemas}
        isConnected
        onOpenTable={onOpenTable}
        onReopenQuery={onReopenQuery}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /audit\.\s*events/ }));
    expect(onOpenTable).toHaveBeenCalledWith("audit", "events");

    fireEvent.click(screen.getByText("select * from users;"));
    expect(onReopenQuery).toHaveBeenCalledWith({
      sql: "select *\nfrom users;",
      connectionId: "conn-1",
    });
  });
});
