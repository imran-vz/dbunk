// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { TablesTab } from "@/components/workspace-overview/tables-tab";
import type { Connection, RelationInfo } from "@/lib/store";

const pgConnection: Connection = {
  id: "conn-pg",
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
  lastSync: "Just now",
  ssl: false,
};

const rel = (overrides: Partial<RelationInfo>): RelationInfo => ({
  schema: overrides.schema ?? "public",
  name: overrides.name ?? "table",
  kind: overrides.kind ?? "table",
  rowCountEstimate: overrides.rowCountEstimate ?? 0,
  totalSizeBytes: overrides.totalSizeBytes ?? 0,
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("TablesTab", () => {
  it("renders PG rows with Rows and Size columns and opens a table on row click", () => {
    const onOpenTable = vi.fn();
    const onLoad = vi.fn().mockResolvedValue(undefined);

    render(
      <TablesTab
        activeConnection={pgConnection}
        schemas={[]}
        relationStats={[
          rel({
            name: "users",
            rowCountEstimate: 12_345,
            totalSizeBytes: 4096,
          }),
          rel({ name: "orders", rowCountEstimate: 7, totalSizeBytes: 256 }),
        ]}
        relationStatsStatus={{ state: "success" }}
        schemaFilter={null}
        onClearSchemaFilter={vi.fn()}
        onLoadRelationStats={onLoad}
        onOpenTable={onOpenTable}
      />,
    );

    expect(screen.getByText("users")).toBeTruthy();
    expect(screen.getByText("orders")).toBeTruthy();
    expect(screen.getByText("Rows (≈)")).toBeTruthy();
    expect(screen.getByText("Size")).toBeTruthy();

    fireEvent.click(screen.getByText("users"));
    expect(onOpenTable).toHaveBeenCalledWith("public", "users");
  });

  it("hides Rows and Size columns on non-PG engines and falls back to schemaExplorer", () => {
    render(
      <TablesTab
        activeConnection={mysqlConnection}
        schemas={[
          { name: "reports", tables: ["events"], views: ["weekly_view"] },
        ]}
        relationStats={undefined}
        relationStatsStatus={undefined}
        schemaFilter={null}
        onClearSchemaFilter={vi.fn()}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onOpenTable={vi.fn()}
      />,
    );

    expect(screen.queryByText("Rows (≈)")).toBeNull();
    expect(screen.queryByText("Size")).toBeNull();
    expect(screen.getByText("events")).toBeTruthy();
    expect(screen.getByText("weekly_view")).toBeTruthy();
  });

  it("applies the schemaFilter prop and clears it when the chip is clicked", () => {
    const onClear = vi.fn();

    render(
      <TablesTab
        activeConnection={pgConnection}
        schemas={[]}
        relationStats={[
          rel({ schema: "public", name: "users" }),
          rel({ schema: "audit", name: "events" }),
        ]}
        relationStatsStatus={{ state: "success" }}
        schemaFilter="audit"
        onClearSchemaFilter={onClear}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onOpenTable={vi.fn()}
      />,
    );

    expect(screen.queryByText("users")).toBeNull();
    expect(screen.getByText("events")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Schema: audit/ }));
    expect(onClear).toHaveBeenCalled();
  });

  it("filters by free-text search across schema and name", () => {
    render(
      <TablesTab
        activeConnection={pgConnection}
        schemas={[]}
        relationStats={[
          rel({ schema: "public", name: "users" }),
          rel({ schema: "audit", name: "events" }),
        ]}
        relationStatsStatus={{ state: "success" }}
        schemaFilter={null}
        onClearSchemaFilter={vi.fn()}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onOpenTable={vi.fn()}
      />,
    );

    const input = screen.getByPlaceholderText("Search tables…");
    fireEvent.change(input, { target: { value: "audit" } });

    expect(screen.queryByText("users")).toBeNull();
    expect(screen.getByText("events")).toBeTruthy();
  });
});
