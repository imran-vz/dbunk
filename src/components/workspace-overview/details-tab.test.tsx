// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { DetailsTab } from "@/components/workspace-overview/details-tab";
import type { Connection, ServerDetails } from "@/lib/store";

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

const details: ServerDetails = {
  serverVersion: "PostgreSQL 16.2 on x86_64-pc-linux-gnu",
  encoding: "UTF8",
  locale: "en_US.UTF-8",
  timezone: "UTC",
  settings: [
    {
      name: "max_connections",
      setting: "100",
      unit: null,
      category: "Connections and Authentication",
      shortDesc: "Sets the maximum number of concurrent connections.",
      source: "configuration file",
      bootVal: "100",
      resetVal: "100",
    },
    {
      name: "shared_buffers",
      setting: "128",
      unit: "MB",
      category: "Resource Usage",
      shortDesc: "Sets the number of shared memory buffers used by the server.",
      source: "default",
      bootVal: "128",
      resetVal: "128",
    },
  ],
  extensions: [
    {
      name: "pg_stat_statements",
      version: "1.10",
      schema: "public",
      description: "track planning and execution statistics",
    },
  ],
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("DetailsTab", () => {
  it("renders the summary panel with the trimmed server version", () => {
    render(
      <DetailsTab
        activeConnection={pgConnection}
        details={details}
        status={{ state: "success" }}
        onLoad={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    // Version is trimmed before the " on " platform suffix.
    expect(screen.getByText("PostgreSQL 16.2")).toBeTruthy();
    expect(screen.getByText("UTF8")).toBeTruthy();
    expect(screen.getByText("UTC")).toBeTruthy();
  });

  it("renders settings grouped by category and flags non-default rows as modified", () => {
    render(
      <DetailsTab
        activeConnection={pgConnection}
        details={details}
        status={{ state: "success" }}
        onLoad={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    expect(screen.getByText("Connections and Authentication")).toBeTruthy();
    expect(screen.getByText("Resource Usage")).toBeTruthy();
    // max_connections has source = "configuration file" → "modified" badge.
    expect(screen.getByText("modified")).toBeTruthy();
  });

  it("filters settings by free-text search and the modified-only chip", () => {
    render(
      <DetailsTab
        activeConnection={pgConnection}
        details={details}
        status={{ state: "success" }}
        onLoad={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    fireEvent.change(
      screen.getByPlaceholderText("Search by name, category, or description…"),
      { target: { value: "shared" } },
    );
    expect(screen.queryByText("max_connections")).toBeNull();
    expect(screen.getByText("shared_buffers")).toBeTruthy();

    // Reset the search, then toggle "Modified only" — shared_buffers
    // (source = "default") should drop out.
    fireEvent.change(
      screen.getByPlaceholderText("Search by name, category, or description…"),
      { target: { value: "" } },
    );
    fireEvent.click(screen.getByRole("button", { name: "Modified only" }));
    expect(screen.getByText("max_connections")).toBeTruthy();
    expect(screen.queryByText("shared_buffers")).toBeNull();
  });

  it("renders the installed-extensions table", () => {
    render(
      <DetailsTab
        activeConnection={pgConnection}
        details={details}
        status={{ state: "success" }}
        onLoad={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    expect(screen.getByText("Installed extensions")).toBeTruthy();
    expect(screen.getByText("pg_stat_statements")).toBeTruthy();
    expect(screen.getByText("1.10")).toBeTruthy();
  });

  it("shows the error banner when the load fails", () => {
    render(
      <DetailsTab
        activeConnection={pgConnection}
        details={undefined}
        status={{ state: "error", error: "connect refused" }}
        onLoad={vi.fn().mockResolvedValue(undefined)}
      />,
    );

    expect(screen.getByText(/Failed to load server details/)).toBeTruthy();
    expect(screen.getByText(/connect refused/)).toBeTruthy();
  });
});
