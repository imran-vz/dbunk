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

import {
  type Connection,
  type TableStructure,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import type {
  BrowseFilter,
  BrowseSortKey,
  BrowseTableResult,
} from "@/lib/table-browse";
import {
  browseTable,
  closeTableBrowseForTab,
  countTableBrowseRows,
  loadTableGridPrefs,
  saveTableGridPrefs,
  type TableBrowseClientResult,
} from "@/lib/table-browse-client";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);
const mockedBrowseTable = vi.mocked(browseTable);
const mockedCountTableBrowseRows = vi.mocked(countTableBrowseRows);
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
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue(undefined);
  mockedBrowseTable.mockReset();
  mockedBrowseTable.mockResolvedValue(okResult());
  mockedCountTableBrowseRows.mockReset();
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

  it("does not let delayed A clear or overwrite C after B has completed", async () => {
    const requestA = deferred<TableBrowseClientResult<BrowseTableResult>>();
    const requestB = deferred<TableBrowseClientResult<BrowseTableResult>>();
    const requestC = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable
      .mockReturnValueOnce(requestA.promise)
      .mockReturnValueOnce(requestB.promise)
      .mockReturnValueOnce(requestC.promise);

    const browseA = useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 1);
    const browseB = useAppStore.getState().refreshTableBrowse("tab-1");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 2);

    requestB.resolve(okResult({ requestId: 2, rows: [["from-b"]] }));
    await browseB;
    expect(tabState()?.nextRequestToken).toBe(2);
    expect(tabState()?.inflightRequestId).toBeNull();

    const browseC = useAppStore.getState().refreshTableBrowse("tab-1");
    await waitUntil(() => mockedBrowseTable.mock.calls.length === 3);
    expect(tabState()?.nextRequestToken).toBe(3);
    expect(tabState()?.inflightRequestId).toBe(3);

    requestA.resolve({ kind: "superseded" });
    await browseA;
    expect(tabState()?.inflightRequestId).toBe(3);
    expect(tabState()?.loadStatus).toEqual({ state: "loading" });
    expect(tabState()?.result?.rows).toEqual([["from-b"]]);

    requestC.resolve(okResult({ requestId: 3, rows: [["from-c"]] }));
    await browseC;
    expect(tabState()?.result?.rows).toEqual([["from-c"]]);
    expect(tabState()?.inflightRequestId).toBeNull();
  });
});

describe("table browse navigation commits", () => {
  it.each([
    ["cancelled", { kind: "cancelled" } as const],
    [
      "error",
      {
        kind: "error",
        error: { kind: "connectionLost" },
      } as const,
    ],
  ])(
    "keeps page 1 after a page-2 %s and requests page 2 again",
    async (_label, failedResult) => {
      mockedBrowseTable.mockResolvedValueOnce(
        okResult({
          pageInfo: {
            mode: "keyset",
            page: 1,
            hasMore: true,
            nextCursor: { values: ["1"] },
          },
        }),
      );
      await useAppStore
        .getState()
        .openTableBrowse("tab-1", "conn-1", "public", "users");

      const failed = deferred<TableBrowseClientResult<BrowseTableResult>>();
      mockedBrowseTable.mockReturnValueOnce(failed.promise);
      const firstNext = useAppStore.getState().goToTableBrowseNextPage("tab-1");
      await waitUntil(() => mockedBrowseTable.mock.calls.length === 2);
      expect(tabState()?.page).toBe(1);
      expect(tabState()?.cursorStack).toEqual([]);

      failed.resolve(failedResult);
      await firstNext;
      expect(tabState()?.page).toBe(1);
      expect(tabState()?.cursorStack).toEqual([]);
      expect(tabState()?.result?.rows).toEqual([["1"]]);

      mockedBrowseTable.mockResolvedValueOnce(
        okResult({
          requestId: 3,
          rows: [["2"]],
          pageInfo: {
            mode: "keyset",
            page: 2,
            hasMore: false,
            nextCursor: null,
          },
        }),
      );
      await useAppStore.getState().goToTableBrowseNextPage("tab-1");

      expect(mockedBrowseTable.mock.calls[2]?.[0].pageRequest).toEqual({
        kind: "keyset",
        cursor: { values: ["1"] },
      });
      expect(tabState()?.page).toBe(2);
      expect(tabState()?.cursorStack).toEqual([{ values: ["1"] }]);
      expect(tabState()?.result?.rows).toEqual([["2"]]);
    },
  );
});

describe("table browse refresh semantics", () => {
  it("refreshes descriptors only for explicit and post-DDL refreshes", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    expect(mockedBrowseTable.mock.calls[0]?.[0].refreshStructure).toBe(false);

    await useAppStore
      .getState()
      .setTableBrowseFilters("tab-1", [eqFilter("1")]);
    expect(mockedBrowseTable.mock.calls[1]?.[0].refreshStructure).toBe(false);

    await useAppStore.getState().refreshTableBrowse("tab-1");
    expect(mockedBrowseTable.mock.calls[2]?.[0].refreshStructure).toBe(true);

    await useAppStore
      .getState()
      .refreshTableBrowsesForRelation("conn-1", "public", "users", {
        refreshStructure: true,
      });
    expect(mockedBrowseTable.mock.calls[3]?.[0].refreshStructure).toBe(true);

    await useAppStore.getState().goToTableBrowseFirstPage("tab-1");
    expect(mockedBrowseTable.mock.calls[4]?.[0].refreshStructure).toBe(false);
  });

  it("invalidates exact counts on refresh but retains them for navigation", async () => {
    mockedBrowseTable.mockResolvedValue(
      okResult({
        pageInfo: {
          mode: "offset",
          page: 1,
          hasMore: true,
          nextCursor: null,
        },
      }),
    );
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    mockedCountTableBrowseRows.mockResolvedValueOnce({
      kind: "ok",
      value: { kind: "exact", value: 42, requestId: 2 },
    });
    await useAppStore.getState().countTableBrowseRows("tab-1");

    await useAppStore.getState().goToTableBrowsePage("tab-1", 2);
    expect(tabState()?.exactCount?.value).toBe(42);

    const refresh = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable.mockReturnValueOnce(refresh.promise);
    const refreshPromise = useAppStore.getState().refreshTableBrowse("tab-1");
    await waitUntil(() => tabState()?.loadStatus.state === "loading");
    expect(tabState()?.exactCount).toBeNull();
    refresh.resolve(okResult({ requestId: 4 }));
    await refreshPromise;
  });
});

describe("table browse refresh with identity-keyed drafts", () => {
  it("applies refreshed rows even when legacy edit state appears in flight", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    const refresh = deferred<TableBrowseClientResult<BrowseTableResult>>();
    mockedBrowseTable.mockReturnValueOnce(refresh.promise);
    const refreshPromise = useAppStore.getState().refreshTableBrowse("tab-1");
    await waitUntil(() => tabState()?.loadStatus.state === "loading");

    useAppStore
      .getState()
      .setTableCellEdit(
        { connectionId: "conn-1", schema: "public", table: "users" },
        0,
        0,
        "edited",
      );
    refresh.resolve(okResult({ requestId: 2, rows: [["replacement"]] }));
    await refreshPromise;

    expect(tabState()?.result?.rows).toEqual([["replacement"]]);
    expect(tabState()?.appliedRequestId).toBe(2);
    expect(
      useAppStore.getState().tableEdits["conn-1::public::users"]?.[0]?.[0],
    ).toBe("edited");
    expect(tabState()?.loadStatus).toEqual({ state: "success" });
  });

  it("applies import- and seed-style refreshes without dropping draft-neutral state", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    useAppStore
      .getState()
      .setTableCellEdit(
        { connectionId: "conn-1", schema: "public", table: "users" },
        0,
        0,
        "pending",
      );
    mockedBrowseTable.mockResolvedValueOnce(
      okResult({ requestId: 2, rows: [["background-write"]] }),
    );

    // Import and seed both finish by calling the same explicit table refresh.
    await useAppStore.getState().refreshTableBrowse("tab-1");

    expect(tabState()?.result?.rows).toEqual([["background-write"]]);
    expect(
      useAppStore.getState().tableEdits["conn-1::public::users"]?.[0]?.[0],
    ).toBe("pending");
  });
});

describe("table browse superseded results", () => {
  it("sets loadStatus idle when the first request is superseded", async () => {
    mockedBrowseTable.mockResolvedValueOnce({ kind: "superseded" });

    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");

    expect(tabState()?.loadStatus).toEqual({ state: "idle" });
    expect(tabState()?.result).toBeNull();
    expect(tabState()?.inflightRequestId).toBeNull();
  });

  it("restores loadStatus success and keeps previous rows when a later request is superseded", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    expect(tabState()?.loadStatus).toEqual({ state: "success" });

    mockedBrowseTable.mockResolvedValueOnce({ kind: "superseded" });
    await useAppStore.getState().refreshTableBrowse("tab-1");

    expect(tabState()?.loadStatus).toEqual({ state: "success" });
    expect(tabState()?.result?.rows).toEqual([["1"]]);
    expect(tabState()?.inflightRequestId).toBeNull();
  });

  it("sets countStatus idle when a count request is superseded", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    mockedCountTableBrowseRows.mockResolvedValueOnce({ kind: "superseded" });

    await useAppStore.getState().countTableBrowseRows("tab-1");

    expect(tabState()?.countStatus).toEqual({ state: "idle" });
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

  it("persists page size after a successful page-size browse", async () => {
    vi.useFakeTimers();
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();

    await useAppStore.getState().setTableBrowsePageSize("tab-1", 25);
    expect(tabState()?.pageSize).toBe(25);
    expect(tabState()?.prefs.pageSize).toBe(25);
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(399);
    expect(mockedSaveTableGridPrefs).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(mockedSaveTableGridPrefs).toHaveBeenCalledTimes(1);
    expect(mockedSaveTableGridPrefs).toHaveBeenCalledWith(
      "conn-1",
      "public",
      "users",
      expect.objectContaining({ pageSize: 25 }),
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

describe("table browse edit identity", () => {
  const pgConnection: Connection = {
    id: "conn-1",
    name: "Local",
    database: "dbunk",
    status: "Connected",
    engine: "PostgreSQL",
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "admin",
    latency: "10 ms",
    ssl: true,
  };

  const usersStructure: TableStructure = {
    columns: [
      {
        name: "id",
        dataType: "integer",
        nullable: false,
        defaultValue: null,
        isPrimaryKey: true,
        ordinalPosition: 1,
      },
      {
        name: "name",
        dataType: "text",
        nullable: false,
        defaultValue: null,
        isPrimaryKey: false,
        ordinalPosition: 2,
      },
    ],
    primaryKey: ["id"],
    foreignKeys: [],
    indexes: [],
    constraints: [],
    capabilities: {
      columns: true,
      primaryKey: true,
      foreignKeys: true,
      indexes: true,
      constraints: true,
      canInsertRows: true,
      canUpdateRows: true,
      canDeleteRows: true,
      canAlterSchema: true,
      uniquenessGuarantee: "exact",
    },
  };

  const nameColumns = [
    { name: "id", castType: "integer", nullable: false },
    { name: "name", castType: "text", nullable: false },
  ];

  it("commits cell edits using tab B rows when two browse tabs share a table", async () => {
    mockedBrowseTable
      .mockResolvedValueOnce(
        okResult({ columns: nameColumns, rows: [["1", "from-a"]] }),
      )
      .mockResolvedValueOnce(
        okResult({ columns: nameColumns, rows: [["99", "from-b"]] }),
      );

    useAppStore.setState({
      connections: [pgConnection],
      tableStructure: {
        [tableStructureKey("conn-1", "public", "users")]: usersStructure,
      },
    });

    await useAppStore
      .getState()
      .openTableBrowse("tab-a", "conn-1", "public", "users");
    await useAppStore
      .getState()
      .openTableBrowse("tab-b", "conn-1", "public", "users");

    expect(tabState("tab-a")?.result?.rows).toEqual([["1", "from-a"]]);
    expect(tabState("tab-b")?.result?.rows).toEqual([["99", "from-b"]]);

    useAppStore
      .getState()
      .setTableCellEdit(
        { connectionId: "conn-1", schema: "public", table: "users" },
        0,
        1,
        "edited",
      );

    mockedInvoke.mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 5 });
    mockedBrowseTable.mockResolvedValue(okResult({ columns: nameColumns }));

    const outcome = await useAppStore
      .getState()
      .commitTableCellEdits(
        { connectionId: "conn-1", schema: "public", table: "users" },
        "tab-b",
      );

    expect(outcome.kind).toBe("completed");
    expect(mockedInvoke).toHaveBeenCalledWith("commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "id", value: "99" }],
            set: [{ column: "name", value: "edited" }],
          },
        ],
      },
    });
  });

  it("targets the schema and identity of the displayed browse tab", async () => {
    mockedBrowseTable
      .mockResolvedValueOnce(
        okResult({ columns: nameColumns, rows: [["1", "public-user"]] }),
      )
      .mockResolvedValueOnce(
        okResult({
          requestId: 2,
          columns: nameColumns,
          rows: [["99", "audit-user"]],
        }),
      );
    useAppStore.setState({
      connections: [pgConnection],
      tableStructure: {
        [tableStructureKey("conn-1", "public", "users")]: usersStructure,
        [tableStructureKey("conn-1", "audit", "users")]: usersStructure,
      },
    });
    await useAppStore
      .getState()
      .openTableBrowse("tab-public", "conn-1", "public", "users");
    await useAppStore
      .getState()
      .openTableBrowse("tab-audit", "conn-1", "audit", "users");
    useAppStore
      .getState()
      .setTableCellEdit(
        { connectionId: "conn-1", schema: "audit", table: "users" },
        0,
        1,
        "edited-audit",
      );
    mockedInvoke.mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 5 });
    mockedBrowseTable.mockResolvedValue(
      okResult({ requestId: 3, columns: nameColumns }),
    );

    await useAppStore
      .getState()
      .commitTableCellEdits(
        { connectionId: "conn-1", schema: "audit", table: "users" },
        "tab-audit",
      );

    expect(mockedInvoke).toHaveBeenCalledWith("commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "audit",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "id", value: "99" }],
            set: [{ column: "name", value: "edited-audit" }],
          },
        ],
      },
    });
  });

  it("applies insert-triggered refreshes without dropping draft-neutral state", async () => {
    mockedBrowseTable.mockResolvedValueOnce(
      okResult({ columns: nameColumns, rows: [["1", "old"]] }),
    );
    useAppStore.setState({
      connections: [pgConnection],
      tableStructure: {
        [tableStructureKey("conn-1", "public", "users")]: usersStructure,
      },
    });
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", "conn-1", "public", "users");
    useAppStore
      .getState()
      .setTableCellEdit(
        { connectionId: "conn-1", schema: "public", table: "users" },
        0,
        1,
        "pending",
      );
    useAppStore.setState((state) => ({
      tableBrowses: {
        ...state.tableBrowses,
        "tab-1": {
          ...state.tableBrowses["tab-1"],
          exactCount: { kind: "exact", value: 1, requestId: 2 },
        },
      },
    }));
    mockedInvoke.mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 4 });
    mockedBrowseTable.mockResolvedValueOnce(
      okResult({ requestId: 3, columns: nameColumns, rows: [["2", "new"]] }),
    );

    const outcome = await useAppStore
      .getState()
      .insertTableRow(
        { connectionId: "conn-1", schema: "public", table: "users" },
        [{ column: "name", value: "new" }],
      );

    expect(outcome.kind).toBe("completed");
    expect(tabState()?.result?.rows).toEqual([["2", "new"]]);
    expect(tabState()?.exactCount).toBeNull();
    expect(
      useAppStore.getState().tableEdits["conn-1::public::users"]?.[0]?.[1],
    ).toBe("pending");
    expect(mockedBrowseTable.mock.calls.at(-1)?.[0].refreshStructure).toBe(
      false,
    );
  });
});
