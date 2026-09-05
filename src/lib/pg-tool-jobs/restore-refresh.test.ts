/* oxlint-disable anti-slop/no-module-mocking -- Tests control the table browse command boundary. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/table-browse-client", () => ({
  browseTable: vi.fn(),
  cancelTableBrowse: vi.fn(() => Promise.resolve({ cancelRequested: false })),
  closeTableBrowseForTab: vi.fn(() => Promise.resolve()),
  countTableBrowseRows: vi.fn(),
  loadTableGridPrefs: vi.fn(() => Promise.resolve(null)),
  saveTableGridPrefs: vi.fn(() => Promise.resolve()),
  resetTableBrowseClientForTab: vi.fn(),
}));

import type { PgToolJob } from "@/lib/pg-tool-jobs/client";
import { refreshAfterPgRestore } from "@/lib/pg-tool-jobs/restore-refresh";
import type { PgTransferJob } from "@/lib/pg-transfer/client";
import { refreshAfterPgCsvImport } from "@/lib/pg-transfer/refresh";
import { type Connection, useAppStore } from "@/lib/store";
import { tableMutationDraftScope } from "@/lib/store";
import type { BrowseTableResult } from "@/lib/table-browse";
import { browseTable, countTableBrowseRows } from "@/lib/table-browse-client";

const mockedBrowse = vi.mocked(browseTable);
const mockedCount = vi.mocked(countTableBrowseRows);
const initialStoreState = useAppStore.getState();

const connection = {
  id: "conn-1",
  name: "Primary",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "10 ms",
  ssl: false,
  environment: "development",
  safeMode: "disabled",
  readOnly: false,
} satisfies Connection;

const restoreJob = {
  jobId: "restore-1",
  connectionId: connection.id,
  kind: "restore",
  format: "custom",
  fileName: "restore.dump",
  phase: "completed",
  startedAt: "now",
  finishedAt: "now",
  bytesProcessed: null,
  totalBytes: 1,
  toolVersion: null,
  failure: null,
} satisfies PgToolJob;

const importJob = {
  jobId: "import-1",
  connectionId: connection.id,
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  phase: "completed",
  startedAt: "now",
  finishedAt: "now",
  totalBytes: 1,
  bytesProcessed: 1,
  rowsProcessed: 1,
  rowsCommitted: 1,
  failure: null,
} satisfies PgTransferJob;

const browseResult = (requestId: number): BrowseTableResult => ({
  requestId,
  columns: [{ name: "id", castType: "integer", nullable: false }],
  rows: [["1"]],
  identity: { kind: "primaryKey", columns: ["id"] },
  rowIdentity: [["1"]],
  pageInfo: {
    mode: "keyset",
    page: 1,
    hasMore: false,
    nextCursor: null,
  },
  count: { kind: "estimated", value: 1 },
  inspection: { sql: "select id from public.users", params: [] },
  omittedRows: 0,
  truncatedCells: 0,
  runtimeMs: 1,
});

type Deferred<T> = {
  promise: Promise<T>;
  resolve: (value: T) => void;
};

const deferred = <T>(): Deferred<T> => {
  let resolve: (value: T) => void = () => undefined;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({
    connections: [connection],
    loadPgObjectCatalog: vi.fn(() => Promise.resolve("ready" as const)),
    loadRelationStats: vi.fn(() => Promise.resolve()),
    loadDatabaseOverviewStats: vi.fn(() => Promise.resolve()),
    loadTableStructure: vi.fn(() => Promise.resolve()),
    appendConsoleEvent: vi.fn(),
  });
  mockedBrowse.mockReset().mockResolvedValue({
    kind: "ok",
    value: browseResult(1),
  });
  mockedCount.mockReset().mockResolvedValue({
    kind: "ok",
    value: { kind: "exact", value: 2, requestId: 1 },
  });
});

afterEach(() => useAppStore.setState(initialStoreState, true));

describe("refreshAfterPgRestore", () => {
  it("drops zero-change drafts and fences analysis already in flight", async () => {
    const analysis = {
      requestId: 1,
      analysisId: 91,
      columns: [],
      tables: [],
      statement: { kind: "analyzed" as const },
    };
    const handle = useAppStore.getState().openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: connection.id,
      source: { kind: "relation", schema: "public", table: "users" },
    });
    if (!handle) throw new Error("Expected mutation draft handle");

    await refreshAfterPgRestore(restoreJob);

    const state = useAppStore.getState();
    expect(state.mutationDrafts[handle.scope]).toBeUndefined();
    const fencedGeneration = state.mutationDraftGenerations[handle.scope];
    if (fencedGeneration === undefined) {
      throw new Error("Expected fenced mutation draft generation");
    }
    expect(fencedGeneration).toBeGreaterThan(handle.generation);
    expect(state.setMutationDraftAnalysis(handle, analysis)).toBe(false);

    const replacement = state.openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: connection.id,
      source: { kind: "relation", schema: "public", table: "users" },
    });
    expect(replacement).toEqual({
      scope: tableMutationDraftScope("tab-1"),
      generation: fencedGeneration + 1,
    });
    if (!replacement) throw new Error("Expected replacement draft handle");
    expect(
      useAppStore.getState().setMutationDraftAnalysis(replacement, analysis),
    ).toBe(true);
  });

  it("fences old browse work and reruns only an exact count that was requested", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-counted", connection.id, "public", "users");
    await useAppStore
      .getState()
      .openTableBrowse("tab-estimated", connection.id, "public", "events");
    useAppStore.setState((state) => ({
      tableBrowses: {
        ...state.tableBrowses,
        "tab-counted": {
          ...state.tableBrowses["tab-counted"],
          exactCount: { kind: "exact", value: 10, requestId: 4 },
          countStatus: { state: "success" },
        },
      },
    }));
    const countedGeneration =
      useAppStore.getState().tableBrowses["tab-counted"].generation;
    const refresh = deferred<Awaited<ReturnType<typeof browseTable>>>();
    mockedBrowse.mockImplementation((payload) => {
      if (payload.tabId === "tab-counted") return refresh.promise;
      return Promise.resolve({ kind: "ok", value: browseResult(2) });
    });

    const completion = refreshAfterPgRestore(restoreJob);

    expect(useAppStore.getState().tableBrowses["tab-counted"]).toMatchObject({
      generation: countedGeneration + 1,
      exactCount: null,
      countStatus: { state: "idle" },
    });
    expect(mockedCount).not.toHaveBeenCalled();

    refresh.resolve({ kind: "ok", value: browseResult(2) });
    await completion;

    expect(mockedCount).toHaveBeenCalledTimes(1);
    expect(mockedCount.mock.calls[0]?.[0].tabId).toBe("tab-counted");
    expect(
      useAppStore.getState().tableBrowses["tab-counted"].exactCount?.value,
    ).toBe(2);
    expect(
      useAppStore.getState().tableBrowses["tab-estimated"].exactCount,
    ).toBeNull();
  });

  it("does not count a tab that is rebound while its restore refresh is in flight", async () => {
    await useAppStore
      .getState()
      .openTableBrowse("tab-1", connection.id, "public", "users");
    useAppStore.setState((state) => ({
      tableBrowses: {
        ...state.tableBrowses,
        "tab-1": {
          ...state.tableBrowses["tab-1"],
          exactCount: { kind: "exact", value: 10, requestId: 4 },
          countStatus: { state: "success" },
        },
      },
    }));
    const refresh = deferred<Awaited<ReturnType<typeof browseTable>>>();
    mockedBrowse.mockReturnValueOnce(refresh.promise);
    const completion = refreshAfterPgRestore(restoreJob);

    useAppStore.setState((state) => ({
      tableBrowses: {
        ...state.tableBrowses,
        "tab-1": {
          ...state.tableBrowses["tab-1"],
          connectionId: "conn-2",
          table: "other",
          generation: state.tableBrowses["tab-1"].generation + 1,
        },
      },
    }));
    refresh.resolve({ kind: "ok", value: browseResult(2) });
    await completion;

    expect(mockedCount).not.toHaveBeenCalled();
  });
});

describe("refreshAfterPgCsvImport", () => {
  it("preserves staged edits while fencing their stale source", async () => {
    const handle = useAppStore.getState().openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: connection.id,
      source: { kind: "relation", schema: "public", table: "users" },
    });
    if (!handle) throw new Error("Expected mutation draft handle");
    useAppStore.getState().stageMutationDraftInsert(handle.scope, {
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: "Ada" }],
    });

    await refreshAfterPgCsvImport(importJob);

    expect(useAppStore.getState().mutationDrafts[handle.scope]).toMatchObject({
      sourceInvalidated: true,
      changeOrder: [expect.any(String)],
      preview: { state: "idle" },
    });
  });

  it("warns against a blind retry after an unknown commit outcome", async () => {
    await refreshAfterPgCsvImport({
      ...importJob,
      phase: "outcomeUnknown",
      rowsCommitted: null,
      failure: { kind: "outcomeUnknown" },
    });

    expect(useAppStore.getState().appendConsoleEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        message: expect.stringContaining("Inspect the target before retrying"),
      }),
    );
  });
});
