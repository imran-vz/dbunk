/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Store tests mock the table-browse client and query-session channel boundaries. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/table-browse-client", () => ({
  browseTable: vi.fn(),
  cancelTableBrowse: vi.fn(() => Promise.resolve({ cancelRequested: false })),
  closeTableBrowseForTab: vi.fn(() => Promise.resolve()),
  countTableBrowseRows: vi.fn(),
  loadTableGridPrefs: vi.fn(),
  saveTableGridPrefs: vi.fn(() => Promise.resolve()),
  nextTableBrowseRequestId: vi.fn(() => 1),
  resetTableBrowseClientForTab: vi.fn(),
}));

vi.mock("@/lib/query-session-channel", () => ({
  cancelQueryExecution: vi.fn(() => Promise.resolve()),
  closeQuerySessionForTab: vi.fn(() => Promise.resolve()),
  commitQueryTransaction: vi.fn(() => Promise.resolve()),
  executeQuerySession: vi.fn(() => Promise.resolve()),
  openQuerySession: vi.fn(() => Promise.resolve()),
  querySessionChannelsAvailable: vi.fn(() => false),
  refreshQueryTransaction: vi.fn(() => Promise.resolve()),
  rollbackQueryTransaction: vi.fn(() => Promise.resolve()),
  setQuerySessionTransactionIsolation: vi.fn(() => Promise.resolve()),
  setQuerySessionTransactionMode: vi.fn(() => Promise.resolve()),
}));

import { useAppStore } from "@/lib/store";
import type {
  BrowseFilter,
  BrowseSortKey,
  BrowseTableResult,
} from "@/lib/table-browse";
import {
  browseTable,
  closeTableBrowseForTab,
  loadTableGridPrefs,
  saveTableGridPrefs,
  type TableBrowseClientResult,
} from "@/lib/table-browse-client";
import { isTauri } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedBrowseTable = vi.mocked(browseTable);
const mockedCloseTableBrowseForTab = vi.mocked(closeTableBrowseForTab);
const mockedLoadTableGridPrefs = vi.mocked(loadTableGridPrefs);
const mockedSaveTableGridPrefs = vi.mocked(saveTableGridPrefs);

const initialStoreState = useAppStore.getState();

const resetStore = () => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ activeConnectionId: "conn-1" });
};

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
};

const deferred = <T>(): Deferred<T> => {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
};

const makeBrowseResult = (
  overrides: Partial<BrowseTableResult> = {},
): BrowseTableResult => ({
  requestId: 1,
  columns: [{ name: "id", castType: "integer", nullable: false }],
  rows: [["1"]],
  identity: { kind: "primaryKey", columns: ["id"] },
  rowIdentity: [["1"]],
  pageInfo: { mode: "keyset", page: 1, hasMore: false, nextCursor: null },
  count: { kind: "estimated", value: 1 },
  inspection: { sql: "select id from public.users", params: [] },
  omittedRows: 0,
  truncatedCells: 0,
  runtimeMs: 8,
  ...overrides,
});

const okResult = (
  overrides: Partial<BrowseTableResult> = {},
): TableBrowseClientResult<BrowseTableResult> => ({
  kind: "ok",
  value: makeBrowseResult(overrides),
});

const tabState = (tabId = "tab-1") =>
  useAppStore.getState().tableBrowses[tabId];

const waitUntil = async (predicate: () => boolean): Promise<void> => {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (predicate()) return;
    await Promise.resolve();
  }
  expect(predicate()).toBe(true);
};

const eqFilter = (value: string): BrowseFilter => ({
  kind: "comparison",
  column: "id",
  operator: "eq",
  value,
});

const idSort = (
  direction: BrowseSortKey["direction"] = "asc",
): BrowseSortKey => ({
  column: "id",
  direction,
  nulls: "last",
});

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedBrowseTable.mockReset();
  mockedBrowseTable.mockResolvedValue(okResult());
  mockedCloseTableBrowseForTab.mockReset();
  mockedCloseTableBrowseForTab.mockResolvedValue(undefined);
  mockedLoadTableGridPrefs.mockReset();
  mockedLoadTableGridPrefs.mockResolvedValue(null);
  mockedSaveTableGridPrefs.mockReset();
  mockedSaveTableGridPrefs.mockResolvedValue(undefined);
  resetStore();
});

afterEach(() => {
  if (vi.isFakeTimers()) {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  }
  resetStore();
});

describe("table browse newest-request-wins", () => {
  it("keeps only the newer rows when interleaved browse A then B resolve in order", async () => {
    const first = deferred<TableBrowseClientResult<BrowseTableResult>>();
    const second = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const openA = useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 1);

    const openB = useAppStore.getState().refreshTableBrowse("tab-1");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 2);

    first.resolve(okResult({ requestId: 1, rows: [["from-a"]] }));
    await openA;
    second.resolve(okResult({ requestId: 2, rows: [["from-b"]] }));
    await openB;

    expect(tabState()?.result?.rows).toEqual([["from-b"]]);
    expect(tabState()?.result?.requestId).toBe(2);
    expect(tabState()?.loadStatus).toEqual({ state: "success" });
  });
});

describe("table browse superseded results", () => {
  it("stays loading when the first request is superseded", async () => {
    mockedBrowseTable.mockResolvedValueOnce({ kind: "superseded" });

    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    expect(tabState()?.loadStatus).toEqual({ state: "loading" });
    expect(tabState()?.result).toBeNull();
  });

  it("keeps previous rows and does not record an error when a later request is superseded", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    expect(tabState()?.loadStatus).toEqual({ state: "success" });

    mockedBrowseTable.mockResolvedValueOnce({ kind: "superseded" });
    await useAppStore.getState().refreshTableBrowse("tab-1");

    expect(tabState()?.loadStatus.state).not.toBe("error");
    expect(tabState()?.result?.rows).toEqual([["1"]]);
  });
});

describe("table browse generation fencing", () => {
  it("rejects a result after the tab is retargeted", async () => {
    const first = deferred<TableBrowseClientResult<BrowseTableResult>>();
    const second = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);

    const openUsers = useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 1);
    const generationOnUsers = tabState()?.generation;

    const openOrders = useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "orders");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 2);

    first.resolve(okResult({ requestId: 1, rows: [["from-users"]] }));
    await openUsers;

    expect(tabState()?.table).toBe("orders");
    expect(tabState()?.generation).not.toBe(generationOnUsers);
    expect(tabState()?.result).toBeNull();
    expect(tabState()?.loadStatus).toEqual({ state: "loading" });

    second.resolve(okResult({ requestId: 2, rows: [["from-orders"]] }));
    await openOrders;

    expect(tabState()?.result?.rows).toEqual([["from-orders"]]);
    expect(tabState()?.table).toBe("orders");
  });

  it("rejects a result after closeTableBrowsesForConnection", async () => {
    const pending = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable.mockReturnValueOnce(pending.promise);

    const open = useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 1);

    await useAppStore.getState().closeTableBrowsesForConnection("conn-1");
    expect(tabState()).toBeUndefined();
    expect(mockedCloseTableBrowseForTab).toHaveBeenCalledWith(
      "conn-1",
      "tab-1",
    );

    pending.resolve(okResult({ requestId: 1, rows: [["stale"]] }));
    await open;

    expect(tabState()).toBeUndefined();
    expect(useAppStore.getState().tableBrowses).toEqual({});
  });
});

describe("table browse reset semantics", () => {
  it("resets to the first page when filters, sort, or page size change", async () => {
    mockedBrowseTable.mockImplementation(async (payload) => {
      const page =
        payload.pageRequest.kind === "offset" ? payload.pageRequest.page : 1;
      return okResult({
        pageInfo: {
          mode: "offset",
          page,
          hasMore: true,
          nextCursor: null,
        },
      });
    });
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    await useAppStore.getState().goToTableBrowsePage("tab-1", 2);
    expect(tabState()?.page).toBe(2);

    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("1")]);
    expect(tabState()?.page).toBe(1);
    expect(tabState()?.cursorStack).toEqual([]);

    await useAppStore.getState().goToTableBrowsePage("tab-1", 2);
    await useAppStore.getState().setTableBrowseSort("tab-1", [idSort("desc")]);
    expect(tabState()?.page).toBe(1);
    expect(tabState()?.cursorStack).toEqual([]);

    await useAppStore.getState().goToTableBrowsePage("tab-1", 2);
    await useAppStore.getState().setTableBrowsePageSize("tab-1", 25);
    expect(tabState()?.page).toBe(1);
    expect(tabState()?.cursorStack).toEqual([]);
    expect(tabState()?.pageSize).toBe(25);
  });

  it("clears filters, sort, and cursor when retargeting the tab", async () => {
    mockedBrowseTable.mockResolvedValue(
      okResult({
        pageInfo: {
          mode: "keyset",
          page: 1,
          hasMore: true,
          nextCursor: { values: ["10"] },
        },
      }),
    );
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("9")]);
    await useAppStore.getState().setTableBrowseSort("tab-1", [idSort("desc")]);
    await useAppStore.getState().setTableBrowseRawFilter("tab-1", "id > 0");
    await useAppStore.getState().goToTableBrowseNextPage("tab-1");

    expect(tabState()?.typedFilters).not.toEqual([]);
    expect(tabState()?.sort).not.toEqual([]);
    expect(tabState()?.rawFilterText).toBe("id > 0");
    expect(tabState()?.cursorStack.length).toBeGreaterThan(0);

    mockedBrowseTable.mockResolvedValue(okResult({ rows: [["order"]] }));
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "orders");

    expect(tabState()?.table).toBe("orders");
    expect(tabState()?.typedFilters).toEqual([]);
    expect(tabState()?.sort).toEqual([]);
    expect(tabState()?.rawFilterText).toBe("");
    expect(tabState()?.filterMode).toBe("typed");
    expect(tabState()?.cursorStack).toEqual([]);
    expect(tabState()?.page).toBe(1);
  });
});

describe("table browse prefs and history", () => {
  it("applies stored prefs on first browse and persists them after 400ms", async () => {
    vi.useFakeTimers();
    const storedSort = [idSort("desc")];
    const storedFilters = [eqFilter("25")];
    mockedLoadTableGridPrefs.mockResolvedValueOnce({
      version: 1,
      pageSize: 25,
      sort: storedSort,
      typedFilters: storedFilters,
      rawFilterText: "id is not null",
      filterMode: "raw",
      filterHistory: [],
      sortHistory: [],
      presets: [],
    });

    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    expect(tabState()?.pageSize).toBe(25);
    expect(tabState()?.sort).toEqual(storedSort);
    expect(tabState()?.typedFilters).toEqual(storedFilters);
    expect(tabState()?.rawFilterText).toBe("id is not null");
    expect(tabState()?.filterMode).toBe("raw");
    expect(tabState()?.prefsLoaded).toBe(true);
    expect(mockedBrowseTable.mock.calls[0]?.[0]).toMatchObject({
      pageSize: 25,
      sort: storedSort,
      filters: [...storedFilters, { kind: "rawSql", text: "id is not null" }],
    });
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();

    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("26")]);
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(399);
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(mockedSaveTableGridPrefs).toHaveBeenCalledTimes(1);
    expect(mockedSaveTableGridPrefs).toHaveBeenCalledWith(
      "conn-1",
      "public",
      "users",
      expect.objectContaining({
        pageSize: 25,
        sort: storedSort,
        typedFilters: [eqFilter("26")],
        filterMode: "raw",
      }),
    );
  });

  it("caps filter history at 20 after 21 filter applies", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    for (let index = 0; index < 21; index += 1) {
      await useAppStore
        .getState()
        .setTableBrowseFilters("tab-1", [eqFilter(String(index))]);
    }

    const history = tabState()?.prefs.filterHistory ?? [];
    expect(history).toHaveLength(20);
    expect(history[0]?.typedFilters).toEqual([eqFilter("20")]);
    expect(history.at(-1)?.typedFilters).toEqual([eqFilter("1")]);
  });

  it("applies a preset's filters, sort, and mode atomically", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("7")]);
    await useAppStore.getState().setTableBrowseSort("tab-1", [idSort("desc")]);
    await useAppStore.getState().setTableBrowseFilterMode("tab-1", "raw");
    await useAppStore.getState().setTableBrowsePageSize("tab-1", 50);
    await useAppStore.getState().saveTableBrowsePreset("tab-1", "focus");

    await useAppStore.getState().setTableBrowseFilters("tab-1", []);
    await useAppStore.getState().setTableBrowseSort("tab-1", []);
    await useAppStore.getState().setTableBrowseFilterMode("tab-1", "typed");
    await useAppStore.getState().setTableBrowsePageSize("tab-1", 10);

    const pending = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable.mockReturnValueOnce(pending.promise);
    const apply = useAppStore
      .getState()
      .applyTableBrowsePreset("tab-1", "focus");
    await waitUntil(() => mockedBrowseTable.mock.calls.length >= 1);

    expect(tabState()?.typedFilters).toEqual([eqFilter("7")]);
    expect(tabState()?.sort).toEqual([idSort("desc")]);
    expect(tabState()?.filterMode).toBe("raw");
    expect(tabState()?.pageSize).toBe(50);
    expect(tabState()?.page).toBe(1);

    pending.resolve(okResult({ requestId: 9, rows: [["preset"]] }));
    await apply;
    expect(tabState()?.result?.rows).toEqual([["preset"]]);
  });
});

describe("table browse invalidCursor recovery", () => {
  it("recovers to the first page when the first result is invalidCursor", async () => {
    mockedBrowseTable
      .mockResolvedValueOnce({
        kind: "error",
        error: { kind: "invalidCursor" },
      })
      .mockResolvedValueOnce(
        okResult({
          requestId: 2,
          rows: [["recovered"]],
          pageInfo: {
            mode: "keyset",
            page: 1,
            hasMore: false,
            nextCursor: null,
          },
        }),
      );

    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    expect(mockedBrowseTable).toHaveBeenCalledTimes(2);
    expect(mockedBrowseTable.mock.calls[1]?.[0]).toMatchObject({
      pageRequest: { kind: "keyset", cursor: null },
    });
    expect(tabState()?.loadStatus).toEqual({ state: "success" });
    expect(tabState()?.page).toBe(1);
    expect(tabState()?.cursorStack).toEqual([]);
    expect(tabState()?.result?.rows).toEqual([["recovered"]]);
  });
});

describe("table browse filter mode and clear", () => {
  it("does not reset the current page when only the filter mode changes", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    mockedBrowseTable.mockResolvedValueOnce(
      okResult({
        pageInfo: {
          mode: "offset",
          page: 2,
          hasMore: true,
          nextCursor: null,
        },
      }),
    );
    await useAppStore.getState().goToTableBrowsePage("tab-1", 2);
    expect(tabState()?.page).toBe(2);

    await useAppStore.getState().setTableBrowseFilterMode("tab-1", "raw");
    expect(tabState()?.page).toBe(2);
    expect(tabState()?.filterMode).toBe("raw");
  });

  it("clears typed and raw filters in a single browse request", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("1")]);
    await useAppStore.getState().setTableBrowseRawFilter("tab-1", "id > 0");
    mockedBrowseTable.mockClear();

    await useAppStore.getState().clearTableBrowseFilters("tab-1");

    expect(mockedBrowseTable).toHaveBeenCalledTimes(1);
    expect(mockedBrowseTable.mock.calls[0]?.[0]).toMatchObject({
      filters: [],
    });
    expect(tabState()?.typedFilters).toEqual([]);
    expect(tabState()?.rawFilterText).toBe("");
  });
});
