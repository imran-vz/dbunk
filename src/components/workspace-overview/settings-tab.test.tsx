/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SettingsTab } from "@/components/workspace-overview/settings-tab";
import type { Connection } from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);

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
};

beforeEach(() => {
  mockedIsTauri.mockReturnValue(false);
  mockedInvoke.mockReset();
});

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
    expect(screen.getAllByText("Enabled").length).toBeGreaterThan(0);
  });

  it("mirrors the resolved environment, safe mode, and read-only fields", () => {
    render(
      <SettingsTab
        connection={{
          ...pgConnection,
          environment: "production",
          safeMode: "inherit",
          readOnly: true,
        }}
      />,
    );

    expect(screen.getByText("Environment")).toBeTruthy();
    expect(screen.getByText("Production")).toBeTruthy();
    expect(screen.getByText("Safe Mode")).toBeTruthy();
    expect(screen.getByText("Inherit (Strict)")).toBeTruthy();
    expect(screen.getByText("Read-only")).toBeTruthy();
  });

  it("loads and renders recent safety overrides", async () => {
    mockedIsTauri.mockReturnValue(true);
    mockedInvoke.mockResolvedValueOnce([
      {
        command: "execute_ddl",
        classes: ["ddl"],
        occurredAt: new Date().toISOString(),
      },
    ]);

    render(<SettingsTab connection={pgConnection} />);

    expect(await screen.findByText("execute_ddl")).toBeTruthy();
    expect(screen.getByText("ddl")).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith("load_safety_overrides", {
      connectionId: pgConnection.id,
    });
  });

  it("refreshes safety overrides after a successful restore", async () => {
    mockedIsTauri.mockReturnValue(true);
    mockedInvoke
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce({ runtimeMs: 12 })
      .mockResolvedValueOnce([
        {
          command: "run_pg_restore",
          classes: [],
          occurredAt: new Date().toISOString(),
        },
      ]);

    const { container } = render(<SettingsTab connection={pgConnection} />);
    await waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("load_safety_overrides", {
        connectionId: pgConnection.id,
      }),
    );

    const input =
      container.querySelector<HTMLInputElement>('input[type="file"]');
    if (!input) throw new Error("restore input not found");
    const file = new File(["SELECT 1;"], "backup.sql");
    Object.defineProperty(file, "arrayBuffer", {
      value: vi.fn(async () => new TextEncoder().encode("SELECT 1;").buffer),
    });
    fireEvent.change(input, {
      target: { files: [file] },
    });

    expect(await screen.findByText("run_pg_restore")).toBeTruthy();
    expect(
      mockedInvoke.mock.calls.filter(
        ([command]) => command === "load_safety_overrides",
      ),
    ).toHaveLength(2);
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
