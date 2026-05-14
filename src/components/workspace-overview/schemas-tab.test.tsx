// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SchemasTab } from "@/components/workspace-overview/schemas-tab";
import type { Connection } from "@/lib/store";

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
  ssl: true,
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("SchemasTab", () => {
  it("aggregates relationStats into per-schema rows and fires onSelectSchema on click", () => {
    const onSelectSchema = vi.fn();
    const onViewSchemaMap = vi.fn();

    render(
      <SchemasTab
        activeConnection={pgConnection}
        relationStats={[
          {
            schema: "public",
            name: "users",
            kind: "table",
            rowCountEstimate: 100,
            totalSizeBytes: 4096,
          },
          {
            schema: "public",
            name: "orders",
            kind: "table",
            rowCountEstimate: 50,
            totalSizeBytes: 2048,
          },
          {
            schema: "public",
            name: "user_view",
            kind: "view",
            rowCountEstimate: 0,
            totalSizeBytes: 0,
          },
          {
            schema: "audit",
            name: "events",
            kind: "table",
            rowCountEstimate: 999,
            totalSizeBytes: 8192,
          },
          {
            schema: "audit",
            name: "events_mv",
            kind: "materialized view",
            rowCountEstimate: 50,
            totalSizeBytes: 1024,
          },
        ]}
        relationStatsStatus={{ state: "success" }}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onSelectSchema={onSelectSchema}
        onViewSchemaMap={onViewSchemaMap}
      />,
    );

    // Both schemas listed.
    expect(screen.getByText("public")).toBeTruthy();
    expect(screen.getByText("audit")).toBeTruthy();

    fireEvent.click(screen.getByText("audit"));
    expect(onSelectSchema).toHaveBeenCalledWith("audit");
  });

  it("fires onViewSchemaMap from the row map action without selecting the table filter", () => {
    const onSelectSchema = vi.fn();
    const onViewSchemaMap = vi.fn();

    render(
      <SchemasTab
        activeConnection={pgConnection}
        relationStats={[
          {
            schema: "public",
            name: "users",
            kind: "table",
            rowCountEstimate: 100,
            totalSizeBytes: 4096,
          },
        ]}
        relationStatsStatus={{ state: "success" }}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onSelectSchema={onSelectSchema}
        onViewSchemaMap={onViewSchemaMap}
      />,
    );

    fireEvent.click(screen.getByLabelText("View public schema map"));

    expect(onViewSchemaMap).toHaveBeenCalledWith("public");
    expect(onSelectSchema).not.toHaveBeenCalled();
  });

  it("shows the loading state when the cache is missing", () => {
    render(
      <SchemasTab
        activeConnection={pgConnection}
        relationStats={undefined}
        relationStatsStatus={{ state: "loading" }}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onSelectSchema={vi.fn()}
        onViewSchemaMap={vi.fn()}
      />,
    );

    expect(screen.getByText("Loading schemas…")).toBeTruthy();
  });

  it("shows the error state when relation stats fail to load", () => {
    render(
      <SchemasTab
        activeConnection={pgConnection}
        relationStats={undefined}
        relationStatsStatus={{ state: "error", error: "connect refused" }}
        onLoadRelationStats={vi.fn().mockResolvedValue(undefined)}
        onSelectSchema={vi.fn()}
        onViewSchemaMap={vi.fn()}
      />,
    );

    expect(screen.getByText(/Failed to load schemas/)).toBeTruthy();
    expect(screen.getByText(/connect refused/)).toBeTruthy();
  });
});
