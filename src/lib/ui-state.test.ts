/* oxlint-disable anti-slop/no-module-mocking, anti-slop/no-unknown-parameters, anti-slop/require-safety-comment-for-type-assertion -- the Tauri IPC bridge is the module's external boundary; tests fake it and inspect its untyped call payloads. */
// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) => String(error),
}));

import { isTauri, tauriInvoke } from "@/lib/tauri";
import {
  flushUiState,
  initUiState,
  resetUiStateForTests,
  uiGet,
  uiRemove,
  uiRemovePrefix,
  uiSet,
} from "@/lib/ui-state";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);

beforeEach(() => {
  window.localStorage.clear();
  resetUiStateForTests();
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue([]);
});

afterEach(() => {
  resetUiStateForTests();
});

const savedEntries = () =>
  mockedInvoke.mock.calls
    .filter(([command]) => command === "save_ui_state")
    .flatMap(
      ([, args]) =>
        (
          args as {
            payload: { entries: Array<{ key: string; value: string }> };
          }
        ).payload.entries,
    );

describe("ui-state store (P8)", () => {
  it("loads the ui.v1 namespace and serves keys under legacy names", async () => {
    mockedInvoke.mockResolvedValueOnce([
      { key: "ui.v1.panel.query-details.size", value: "340" },
      { key: "ui.v1.migrated", value: "1" },
    ]);
    await initUiState();
    expect(uiGet("dbunk.panel.query-details.size")).toBe("340");
    expect(uiGet("dbunk.unknown")).toBeNull();
  });

  it("batches debounced writes into one save_ui_state call", async () => {
    await initUiState();
    uiSet("dbunk.panel.a.size", "100");
    uiSet("dbunk.panel.b.size", "200");
    uiSet("dbunk.panel.a.size", "120");
    await flushUiState();

    // The first init also stamps the migration marker; ignore it here.
    const entries = savedEntries().filter(
      (entry) => entry.key !== "ui.v1.migrated",
    );
    expect(entries).toEqual([
      { key: "ui.v1.panel.a.size", value: "120" },
      { key: "ui.v1.panel.b.size", value: "200" },
    ]);
    expect(uiGet("dbunk.panel.a.size")).toBe("120");
  });

  it("deletes keys and prefixes through delete_ui_state", async () => {
    mockedInvoke.mockResolvedValueOnce([
      { key: "ui.v1.grid.layout.conn-1.public.users", value: "{}" },
      { key: "ui.v1.grid.layout.conn-2.public.users", value: "{}" },
      { key: "ui.v1.migrated", value: "1" },
    ]);
    await initUiState();

    uiRemovePrefix("dbunk.grid.layout.conn-1.");
    uiRemove("dbunk.grid.layout.conn-2.public.users");
    expect(uiGet("dbunk.grid.layout.conn-1.public.users")).toBeNull();
    expect(uiGet("dbunk.grid.layout.conn-2.public.users")).toBeNull();

    await flushUiState();
    const deleteCall = mockedInvoke.mock.calls.find(
      ([command]) => command === "delete_ui_state",
    );
    expect(deleteCall?.[1]).toEqual({
      payload: {
        keys: ["ui.v1.grid.layout.conn-2.public.users"],
        prefixes: ["ui.v1.grid.layout.conn-1."],
      },
    });
  });

  it("migrates legacy dbunk.* localStorage once, keeping boot-cache keys", async () => {
    window.localStorage.setItem("dbunk.panel.navigator.size", "260");
    window.localStorage.setItem("dbunk.workbench.dock.query-1.open", "true");
    window.localStorage.setItem("dbunk.theme", "dark");
    window.localStorage.setItem("dbunk.density", "compact");

    await initUiState();

    // Imported into the store under ui.v1.*.
    expect(uiGet("dbunk.panel.navigator.size")).toBe("260");
    // Dead key dropped, not imported.
    expect(uiGet("dbunk.workbench.dock.query-1.open")).toBeNull();
    // Legacy values stay in localStorage until the migration batch is
    // durably in SQLite — they must never exist in neither storage.
    expect(window.localStorage.getItem("dbunk.panel.navigator.size")).toBe(
      "260",
    );

    await flushUiState();
    const entries = savedEntries();
    expect(entries).toContainEqual({
      key: "ui.v1.panel.navigator.size",
      value: "260",
    });
    expect(entries).toContainEqual({ key: "ui.v1.migrated", value: "1" });
    // Committed — localStorage now holds only the boot-cache mirrors.
    expect(
      window.localStorage.getItem("dbunk.panel.navigator.size"),
    ).toBeNull();
    expect(
      window.localStorage.getItem("dbunk.workbench.dock.query-1.open"),
    ).toBeNull();
    expect(window.localStorage.getItem("dbunk.theme")).toBe("dark");
    expect(window.localStorage.getItem("dbunk.density")).toBe("compact");
  });

  it("keeps legacy localStorage values when the migration write fails", async () => {
    window.localStorage.setItem("dbunk.panel.navigator.size", "260");
    await initUiState();
    mockedInvoke.mockRejectedValueOnce(new Error("db locked"));
    await flushUiState();

    // The batch never committed, so the legacy copy must survive.
    expect(window.localStorage.getItem("dbunk.panel.navigator.size")).toBe(
      "260",
    );

    // Retry succeeds — now it's safe to remove.
    await flushUiState();
    expect(
      window.localStorage.getItem("dbunk.panel.navigator.size"),
    ).toBeNull();
    expect(savedEntries()).toContainEqual({
      key: "ui.v1.panel.navigator.size",
      value: "260",
    });
  });

  it("does not re-import after the one-shot migration", async () => {
    mockedInvoke.mockResolvedValueOnce([{ key: "ui.v1.migrated", value: "1" }]);
    // A stale legacy key that reappeared after migration.
    window.localStorage.setItem("dbunk.panel.navigator.size", "999");
    await initUiState();
    await flushUiState();
    // Removed from localStorage but not imported over the store.
    expect(
      window.localStorage.getItem("dbunk.panel.navigator.size"),
    ).toBeNull();
    expect(uiGet("dbunk.panel.navigator.size")).toBeNull();
  });

  it("retains failed writes and deletes for the retry flush", async () => {
    await initUiState();
    await flushUiState(); // Commit the migration marker first.
    mockedInvoke.mockClear();

    uiSet("dbunk.panel.a.size", "100");
    uiRemove("dbunk.panel.b.size");
    uiRemovePrefix("dbunk.grid.layout.conn-1.");
    mockedInvoke.mockRejectedValueOnce(new Error("db locked"));
    await flushUiState();

    // A later identical set must not be swallowed by the cache check.
    uiSet("dbunk.panel.a.size", "100");

    mockedInvoke.mockClear();
    await flushUiState();
    expect(savedEntries()).toContainEqual({
      key: "ui.v1.panel.a.size",
      value: "100",
    });
    const deleteCall = mockedInvoke.mock.calls.find(
      ([command]) => command === "delete_ui_state",
    );
    expect(deleteCall?.[1]).toEqual({
      payload: {
        keys: ["ui.v1.panel.b.size"],
        prefixes: ["ui.v1.grid.layout.conn-1."],
      },
    });
  });

  it("prefers values written during a failed flush over the snapshot", async () => {
    await initUiState();
    await flushUiState();
    mockedInvoke.mockClear();

    uiSet("dbunk.panel.a.size", "100");
    mockedInvoke.mockImplementationOnce(async () => {
      // Simulate a newer write landing while the flush is in flight.
      uiSet("dbunk.panel.a.size", "200");
      throw new Error("db locked");
    });
    await flushUiState();

    mockedInvoke.mockClear();
    await flushUiState();
    const entries = savedEntries();
    expect(entries).toContainEqual({ key: "ui.v1.panel.a.size", value: "200" });
    expect(entries).not.toContainEqual({
      key: "ui.v1.panel.a.size",
      value: "100",
    });
  });

  it("drops oversized values instead of wedging the flush queue", async () => {
    await initUiState();
    await flushUiState();
    mockedInvoke.mockClear();
    const errorSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    // The backend rejects any batch containing a >512 KiB entry, so an
    // enqueued oversized value would deterministically fail every retry.
    uiSet("dbunk.session", "x".repeat(600 * 1024));
    uiSet("dbunk.panel.a.size", "100");
    await flushUiState();

    const entries = savedEntries();
    expect(entries).toContainEqual({ key: "ui.v1.panel.a.size", value: "100" });
    expect(entries.some((entry) => entry.key === "ui.v1.session")).toBe(false);
    expect(errorSpy).toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it("serializes overlapping flushes so batches commit in order", async () => {
    await initUiState();
    await flushUiState();
    mockedInvoke.mockClear();

    let releaseFirst = () => {};
    const firstSaveGate = new Promise<void>((resolve) => {
      releaseFirst = resolve;
    });
    let flushB: Promise<void> = Promise.resolve();
    mockedInvoke.mockImplementationOnce(async () => {
      // A newer value lands — and flushes — while save A is in flight.
      uiSet("dbunk.panel.a.size", "200");
      flushB = flushUiState();
      await firstSaveGate;
      return [];
    });

    uiSet("dbunk.panel.a.size", "100");
    const flushA = flushUiState();

    await vi.waitFor(() =>
      expect(
        mockedInvoke.mock.calls.filter(
          ([command]) => command === "save_ui_state",
        ),
      ).toHaveLength(1),
    );
    // Flush B must not issue its save while A's is still pending —
    // concurrent batches can commit out of order and persist the stale
    // value.
    await Promise.resolve();
    await Promise.resolve();
    expect(
      mockedInvoke.mock.calls.filter(
        ([command]) => command === "save_ui_state",
      ),
    ).toHaveLength(1);
    releaseFirst();
    await Promise.all([flushA, flushB]);

    const saves = mockedInvoke.mock.calls.filter(
      ([command]) => command === "save_ui_state",
    );
    expect(saves).toHaveLength(2);
    expect(savedEntries()).toEqual([
      { key: "ui.v1.panel.a.size", value: "100" },
      { key: "ui.v1.panel.a.size", value: "200" },
    ]);
  });

  it("never blocks launch on a storage failure", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("db locked"));
    await initUiState();
    expect(uiGet("dbunk.anything")).toBeNull();
    // Writes still work against the in-memory cache.
    uiSet("dbunk.panel.a.size", "10");
    expect(uiGet("dbunk.panel.a.size")).toBe("10");
  });
});
