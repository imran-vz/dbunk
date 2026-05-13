// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SettingsTab } from "@/components/workspace-overview/settings-tab";
import type { Connection } from "@/lib/store";

const pgConnection: Connection = {
  id: "conn-pg",
  name: "Primary PG",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "db.example.com",
  port: 5432,
  user: "postgres",
  password: "",
  role: "readonly",
  latency: "12 ms",
  lastSync: "Just now",
  ssl: true,
};

const clickhouseConnection: Connection = {
  id: "conn-ch",
  name: "Reports CH",
  database: "default",
  status: "Connected",
  engine: "ClickHouse",
  host: "ch.example.com",
  port: 8443,
  user: "default",
  password: "",
  role: "",
  latency: "20 ms",
  lastSync: "Just now",
  useHttps: true,
  urlPath: "/clickhouse",
};

const sqliteConnection: Connection = {
  id: "conn-sqlite",
  name: "Local SQLite",
  database: "/tmp/local.db",
  status: "Connected",
  engine: "SQLite",
  host: "",
  port: 0,
  user: "",
  password: "",
  role: "",
  latency: "1 ms",
  lastSync: "Just now",
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("SettingsTab", () => {
  it("renders the Postgres connection's stored fields including the SSL row", () => {
    render(<SettingsTab connection={pgConnection} />);

    expect(screen.getByText("Connection settings")).toBeTruthy();
    expect(screen.getByText("Primary PG")).toBeTruthy();
    expect(screen.getByText("PostgreSQL")).toBeTruthy();
    expect(screen.getByText("db.example.com")).toBeTruthy();
    expect(screen.getByText("5432")).toBeTruthy();
    expect(screen.getByText("app")).toBeTruthy();
    expect(screen.getByText("postgres")).toBeTruthy();
    expect(screen.getByText("readonly")).toBeTruthy();
    expect(screen.getByText("SSL")).toBeTruthy();
    expect(screen.getByText("Enabled")).toBeTruthy();
  });

  it("renders ClickHouse-specific HTTPS and URL-path rows", () => {
    render(<SettingsTab connection={clickhouseConnection} />);

    expect(screen.getByText("HTTPS")).toBeTruthy();
    expect(screen.getByText("URL path")).toBeTruthy();
    expect(screen.getByText("/clickhouse")).toBeTruthy();
  });

  it("renders a SQLite connection without TLS rows", () => {
    render(<SettingsTab connection={sqliteConnection} />);

    expect(screen.queryByText("SSL")).toBeNull();
    expect(screen.queryByText("HTTPS")).toBeNull();
    expect(screen.queryByText("URL path")).toBeNull();
    expect(screen.getByText("SQLite")).toBeTruthy();
  });

  it("opens the edit dialog when Edit connection is clicked", () => {
    render(<SettingsTab connection={pgConnection} />);

    // Dialog body is not in the DOM until the button is clicked.
    expect(screen.queryByText(/Update the connection details/)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Edit connection/ }));

    expect(screen.getByText(/Update the connection details/)).toBeTruthy();
  });
});
