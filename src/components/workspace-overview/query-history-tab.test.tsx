// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { QueryHistoryTab } from "@/components/workspace-overview/query-history-tab";
import type { Connection, QueryHistoryEntry } from "@/lib/store";

const pgConnection: Connection = {
  id: "conn-1",
  name: "Primary",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "db.example.com",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  latency: "12 ms",
  lastSync: "Just now",
  ssl: true,
};

const entry = (overrides: Partial<QueryHistoryEntry>): QueryHistoryEntry => ({
  id: overrides.id ?? "id",
  sql: overrides.sql ?? "select 1",
  connectionId: overrides.connectionId ?? "conn-1",
  connectionName: overrides.connectionName ?? "Primary",
  database: overrides.database ?? "app",
  engine: overrides.engine ?? "PostgreSQL",
  status: overrides.status ?? "success",
  errorMessage: overrides.errorMessage,
  runtimeMs: overrides.runtimeMs ?? 4,
  rowCount: overrides.rowCount,
  startedAt: overrides.startedAt ?? "2026-05-13T12:00:00.000Z",
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("QueryHistoryTab", () => {
  it("scopes to the current connection by default and toggles to all connections", () => {
    const history: QueryHistoryEntry[] = [
      entry({ id: "h1", sql: "select * from users", connectionId: "conn-1" }),
      entry({
        id: "h2",
        sql: "select * from orders",
        connectionId: "conn-2",
        connectionName: "Reporting",
      }),
    ];

    render(
      <QueryHistoryTab
        activeConnection={pgConnection}
        queryHistory={history}
        onReopenEntry={vi.fn()}
      />,
    );

    expect(screen.getByText("select * from users")).toBeTruthy();
    expect(screen.queryByText("select * from orders")).toBeNull();

    fireEvent.click(
      screen.getByRole("button", { name: /Showing this connection/ }),
    );
    expect(screen.getByText("select * from orders")).toBeTruthy();
  });

  it("filters by the search text against the SQL column", () => {
    const history: QueryHistoryEntry[] = [
      entry({ id: "h1", sql: "select * from users where id = 1" }),
      entry({ id: "h2", sql: "update orders set status = 'paid'" }),
    ];

    render(
      <QueryHistoryTab
        activeConnection={pgConnection}
        queryHistory={history}
        onReopenEntry={vi.fn()}
      />,
    );

    const input = screen.getByPlaceholderText("Search SQL…");
    fireEvent.change(input, { target: { value: "orders" } });

    expect(screen.queryByText(/from users/)).toBeNull();
    expect(screen.getByText(/update orders/)).toBeTruthy();
  });

  it("filters by status when the Errors chip is selected", () => {
    const history: QueryHistoryEntry[] = [
      entry({ id: "h1", sql: "select 1", status: "success" }),
      entry({
        id: "h2",
        sql: "select * from missing",
        status: "error",
        errorMessage: "relation missing does not exist",
      }),
    ];

    render(
      <QueryHistoryTab
        activeConnection={pgConnection}
        queryHistory={history}
        onReopenEntry={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Errors" }));
    expect(screen.queryByText("select 1")).toBeNull();
    expect(screen.getByText("select * from missing")).toBeTruthy();
  });

  it("calls onReopenEntry when the Open-in-editor button is clicked", () => {
    const handleReopen = vi.fn();
    const target = entry({ id: "h1", sql: "select 1" });

    render(
      <QueryHistoryTab
        activeConnection={pgConnection}
        queryHistory={[target]}
        onReopenEntry={handleReopen}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Open in editor" }));
    expect(handleReopen).toHaveBeenCalledWith(target);
  });

  it("shows an empty-state when no entries match the filters", () => {
    const history: QueryHistoryEntry[] = [entry({ id: "h1", sql: "select 1" })];

    render(
      <QueryHistoryTab
        activeConnection={pgConnection}
        queryHistory={history}
        onReopenEntry={vi.fn()}
      />,
    );

    const input = screen.getByPlaceholderText("Search SQL…");
    fireEvent.change(input, { target: { value: "nothing-matches" } });

    expect(
      screen.getByText("No queries match the current filters."),
    ).toBeTruthy();
  });
});
