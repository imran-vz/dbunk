/* oxlint-disable anti-slop/no-module-mocking -- The desktop boundary is unavailable in jsdom. */
// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/tauri")>()),
  isTauri: () => true,
}));

import { pgToolObserver } from "@/lib/pg-tool-jobs/observer";
import { type Connection, useAppStore } from "@/lib/store";

import { PgToolWorkspace } from "./workspace";

const connection: Connection = {
  id: "conn-1",
  name: "Local",
  database: "postgres",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "4 ms",
  ssl: false,
};

const initialStore = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStore, true);
  useAppStore.setState({ connections: [connection] });
  pgToolObserver.store.setState({
    jobs: [],
    error: null,
    refreshing: true,
    observedAt: null,
  });
});

afterEach(() => {
  cleanup();
  pgToolObserver.store.setState({
    jobs: [],
    error: null,
    refreshing: false,
    observedAt: null,
  });
  vi.restoreAllMocks();
});

describe("PostgreSQL tool workspace", () => {
  it("keeps setup editable during routine observation polls", () => {
    render(<PgToolWorkspace connection={connection} />);

    expect(
      screen
        .getByRole("combobox", { name: "Archive format" })
        .matches(":disabled"),
    ).toBe(false);
    expect(
      screen.getByRole("button", { name: /Browse/ }).matches(":disabled"),
    ).toBe(false);
  });
});
