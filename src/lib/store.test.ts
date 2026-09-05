/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-runtime-typeof anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { pgToolClient } from "@/lib/pg-tool-jobs/client";

const { mockedRequestConfirm } = vi.hoisted(() => ({
  mockedRequestConfirm: vi.fn(() => Promise.resolve(true)),
}));
vi.mock("@/lib/confirm", () => ({
  requestConfirm: mockedRequestConfirm,
  requestPrompt: vi.fn(() => Promise.resolve(null)),
}));

vi.mock("@/lib/ui-state", () => {
  const memory = new Map<string, string>();
  return {
    initUiState: () => Promise.resolve(),
    isUiStateReady: () => true,
    flushUiState: () => Promise.resolve(),
    uiGet: (key: string) => memory.get(key) ?? null,
    uiSet: (key: string, value: string) => {
      memory.set(key, value);
    },
    uiRemove: (key: string) => {
      memory.delete(key);
    },
    uiRemovePrefix: (prefix: string) => {
      const doomed: string[] = [];
      for (const key of memory.keys()) {
        if (key.startsWith(prefix)) doomed.push(key);
      }
      for (const key of doomed) memory.delete(key);
    },
    resetUiStateForTests: () => {
      memory.clear();
    },
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
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

vi.mock("@/lib/table-browse-client", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/lib/table-browse-client")>();
  return {
    ...actual,
    closeTableBrowseForTab: vi.fn(() => Promise.resolve()),
  };
});

vi.mock("@/lib/result-mutation-client", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/lib/result-mutation-client")>();
  return {
    ...actual,
    closeResultMutationForConnection: vi.fn(() => Promise.resolve()),
  };
});

import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import { querySessionChannelsAvailable } from "@/lib/query-session-channel";
import type {
  AnalyzeResultSetResult,
  PreviewResult,
} from "@/lib/result-mutation";
import {
  type Connection,
  type ManagedServerWithStatus,
  type PgObjectCatalog,
  type QueryOutcome,
  type StructureChange,
  pgObjectDescriptionKey,
  tableMutationDraftScope,
  tableDataKey,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { defaultTableGridPrefs } from "@/lib/table-browse";
import { closeTableBrowseForTab } from "@/lib/table-browse-client";
import { newTableDesignerForm } from "@/lib/table-designer";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedIsTauri = vi.mocked(isTauri);
const mockedChannelsAvailable = vi.mocked(querySessionChannelsAvailable);
const mockedCloseTableBrowseForTab = vi.mocked(closeTableBrowseForTab);

const initialStoreState = useAppStore.getState();

const resetStore = () => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ activeConnectionId: "conn-1" });
};

const emptyPgObjectCatalog = () => ({
  schemas: [],
  eventTriggers: [],
  roles: [],
  tablespaces: [],
  truncated: [],
});

beforeEach(() => {
  vi.spyOn(pgToolClient, "list").mockResolvedValue([]);
  mockedIsTauri.mockReturnValue(true);
  mockedChannelsAvailable.mockReturnValue(false);
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue(undefined);
  mockedCloseTableBrowseForTab.mockReset();
  mockedCloseTableBrowseForTab.mockResolvedValue(undefined);
  resetStore();
});

afterEach(() => {
  resetStore();
});

/**
 * Common assertion for early-validation `failed` paths in
 * `commitStructureChanges`: no backend call, no lifecycle slot left
 * behind, and the failure reason matches.
 */
const expectCommitStructureFailure = async (
  key: string,
  reasonMatch: RegExp,
): Promise<void> => {
  const outcome = await useAppStore.getState().commitStructureChanges(key);
  expect(mockedInvoke).not.toHaveBeenCalled();
  expect(useAppStore.getState().structureCommitStatus[key]).toBeUndefined();
  if (outcome.kind !== "failed") {
    throw new Error(`expected failed outcome, got ${outcome.kind}`);
  }
  expect(outcome.reason).toMatch(reasonMatch);
};

const seedQueryTab = (overrides?: { id?: string; query?: string }) => {
  const id = overrides?.id ?? "tab-1";
  const query = overrides?.query ?? "select 1";
  useAppStore.setState({
    workspaceTabs: [
      {
        id,
        kind: "query",
        label: "query_1.sql",
        connectionId: "conn-1",
        schema: "public",
        query,
      },
    ],
    activeConnectionId: "conn-1",
  });
  return id;
};

describe("store.loadTableData", () => {
  it("invokes load_table_data with default page and pageSize", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id"],
      rows: [["1"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 5,
    });

    await useAppStore.getState().loadTableData("conn-1", "public", "users");

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 1,
        pageSize: 100,
      },
    });
  });

  it("forwards explicit page and pageSize arguments", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      page: 3,
      pageSize: 25,
      totalRows: 200,
      runtimeMs: 1,
    });

    await useAppStore
      .getState()
      .loadTableData("conn-1", "public", "users", 3, 25);

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 3,
        pageSize: 25,
      },
    });
  });

  it("populates tableData on success and marks status as success", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id", "name"],
      rows: [["1", "alice"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 7,
    });

    await useAppStore.getState().loadTableData("conn-1", "public", "users");

    const key = tableDataKey("conn-1", "public", "users");
    const state = useAppStore.getState();
    expect(state.tableData[key]).toMatchObject({
      columns: ["id", "name"],
      rows: [["1", "alice"]],
      page: 1,
      pageSize: 100,
      totalRows: 1,
      runtimeMs: 7,
    });
    expect(state.tableLoadStatus[key]).toEqual({ state: "success" });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("connection lost"));

    await useAppStore.getState().loadTableData("conn-1", "public", "users");

    const key = tableDataKey("conn-1", "public", "users");
    const status = useAppStore.getState().tableLoadStatus[key];
    if (status.state !== "error") {
      throw new Error(`expected error status, got ${status.state}`);
    }
    expect(status.error).toContain("connection lost");
  });

  it("sets loading status before resolving", async () => {
    let resolveInvoke: ((value: unknown) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve as (value: unknown) => void;
        }),
    );

    const promise = useAppStore
      .getState()
      .loadTableData("conn-1", "public", "users");

    const key = tableDataKey("conn-1", "public", "users");
    expect(useAppStore.getState().tableLoadStatus[key]?.state).toBe("loading");

    resolveInvoke?.({
      columns: [],
      rows: [],
      page: 1,
      pageSize: 100,
      totalRows: 0,
      runtimeMs: 1,
    });
    await promise;

    expect(useAppStore.getState().tableLoadStatus[key]?.state).toBe("success");
  });
});

describe("store.refreshTableData", () => {
  it("re-fetches using stored page and pageSize", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id"],
      rows: [["1"]],
      page: 2,
      pageSize: 25,
      totalRows: 50,
      runtimeMs: 2,
    });

    await useAppStore
      .getState()
      .loadTableData("conn-1", "public", "users", 2, 25);

    mockedInvoke.mockResolvedValueOnce({
      columns: ["id"],
      rows: [["2"]],
      page: 2,
      pageSize: 25,
      totalRows: 50,
      runtimeMs: 3,
    });

    const key = tableDataKey("conn-1", "public", "users");
    await useAppStore.getState().refreshTableData(key);

    expect(mockedInvoke).toHaveBeenLastCalledWith("load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 2,
        pageSize: 25,
      },
    });
  });

  it("is a no-op when there is no prior data for the key", async () => {
    await useAppStore.getState().refreshTableData("missing-key");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("store.openTableTab", () => {
  it("keeps same-named tables in different schemas as distinct tabs", () => {
    useAppStore.getState().openWorkspaceTab({
      kind: "table",
      label: "users",
      connectionId: "conn-1",
      schema: "public",
      table: "users",
    });
    const publicTabId = useAppStore.getState().activeTabId;
    useAppStore.getState().openWorkspaceTab({
      kind: "table",
      label: "users",
      connectionId: "conn-1",
      schema: "audit",
      table: "users",
    });

    const tabs = useAppStore.getState().workspaceTabs;
    expect(tabs).toHaveLength(2);
    expect(tabs.map((tab) => [tab.schema, tab.table])).toEqual([
      ["public", "users"],
      ["audit", "users"],
    ]);
    expect(useAppStore.getState().activeTabId).not.toBe(publicTabId);
  });

  it("continues deduplicating query tabs by label and connection", () => {
    const query = {
      kind: "query" as const,
      label: "query_1.sql",
      connectionId: "conn-1",
      schema: "public",
      query: "select 1",
    };
    useAppStore.getState().openWorkspaceTab(query);
    useAppStore.getState().openWorkspaceTab({
      ...query,
      schema: "audit",
      query: "select 2",
    });
    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
  });

  it("quotes relation navigation and deduplicates it by schema-qualified identity", () => {
    useAppStore.getState().openViewTab('odd"schema', 'shared"view');
    const firstTabId = useAppStore.getState().activeTabId;
    useAppStore.getState().openViewTab("audit", 'shared"view');
    useAppStore.getState().openViewTab('odd"schema', 'shared"view');

    const tabs = useAppStore.getState().workspaceTabs;
    expect(tabs).toHaveLength(2);
    expect(tabs.map((tab) => tab.relationRef)).toEqual([
      { schema: 'odd"schema', name: 'shared"view' },
      { schema: "audit", name: 'shared"view' },
    ]);
    expect(tabs[0]?.query).toBe(
      'select * from "odd""schema"."shared""view" limit 100;',
    );
    expect(useAppStore.getState().activeTabId).toBe(firstTabId);
  });

  it("invokes load_table_data and not run_query", async () => {
    mockedInvoke.mockResolvedValue({
      columns: [],
      rows: [],
      page: 1,
      pageSize: 100,
      totalRows: 0,
      runtimeMs: 0,
    });

    useAppStore.getState().openTableTab("public", "users");

    // openTableTab schedules loadTableData via void, await microtasks.
    await new Promise((resolve) => setTimeout(resolve, 0));

    const calls = mockedInvoke.mock.calls.map((call) => call[0]);
    expect(calls).toContain("load_table_data");
    expect(calls).not.toContain("run_query");
  });

  it("opens PostgreSQL table tabs through browse_table_data", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "12 ms",
          ssl: true,
        },
      ],
      activeConnectionId: "conn-1",
    });
    mockedInvoke.mockImplementation((command) => {
      if (command === "load_table_grid_prefs") return Promise.resolve(null);
      if (command === "browse_table_data") {
        return Promise.resolve({
          requestId: 1,
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
          inspection: { sql: "SELECT 1", params: [] },
          omittedRows: 0,
          truncatedCells: 0,
          runtimeMs: 1,
        });
      }
      return Promise.resolve(undefined);
    });

    useAppStore.getState().openTableTab("public", "users");
    await new Promise((resolve) => setTimeout(resolve, 0));
    for (let attempt = 0; attempt < 20; attempt += 1) {
      await Promise.resolve();
    }

    const calls = mockedInvoke.mock.calls.map((call) => call[0]);
    expect(calls).toContain("browse_table_data");
    expect(calls).not.toContain("load_table_data");
    expect(calls).not.toContain("run_query");
  });
});

describe("store.openTableDesignerTab", () => {
  it("opens a fresh draft for every New table action in the same schema", () => {
    useAppStore.getState().openTableDesignerTab("public");
    const firstTabId = useAppStore.getState().activeTabId;
    useAppStore.getState().updateTableDesignerDraft(firstTabId, (draft) => ({
      ...draft,
      name: "orders",
    }));

    useAppStore.getState().openTableDesignerTab("public");

    const state = useAppStore.getState();
    expect(state.workspaceTabs).toHaveLength(2);
    expect(state.activeTabId).not.toBe(firstTabId);
    expect(
      state.workspaceTabs.map((tab) => tab.tableDesignerDraft?.name),
    ).toEqual(["orders", ""]);
  });
});

describe("store.openObjectTab", () => {
  it("deduplicates by canonical typed reference", () => {
    useAppStore.getState().openObjectTab({
      kind: "schema",
      schema: null,
      name: "lifecycle",
      identityArgs: null,
    });
    const firstId = useAppStore.getState().activeTabId;
    useAppStore.getState().openObjectTab({
      kind: "schema",
      schema: null,
      name: "lifecycle",
      identityArgs: null,
    });

    const tabs = useAppStore.getState().workspaceTabs;
    expect(tabs).toHaveLength(1);
    expect(tabs[0]).toEqual(
      expect.objectContaining({
        id: firstId,
        kind: "object",
        label: "lifecycle",
        schema: "",
      }),
    );
  });

  it("keeps overloaded routines as distinct object tabs", () => {
    useAppStore.getState().openObjectTab({
      kind: "function",
      schema: "public",
      name: "add_nums",
      identityArgs: "integer, integer",
    });
    useAppStore.getState().openObjectTab({
      kind: "function",
      schema: "public",
      name: "add_nums",
      identityArgs: "numeric, numeric",
    });

    expect(
      useAppStore.getState().workspaceTabs.map((tab) => tab.label),
    ).toEqual([
      "public.add_nums(integer, integer)",
      "public.add_nums(numeric, numeric)",
    ]);
  });
});

describe("runQuery status tracking", () => {
  it("sets running lifecycle in-flight, clears on success, returns completed outcome", async () => {
    const tabId = seedQueryTab();

    let resolveInvoke: ((value: unknown) => void) | null = null;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve;
        }),
    );

    let outcomePromise!: Promise<QueryOutcome>;
    await act(async () => {
      outcomePromise = useAppStore.getState().runQuery(tabId);
    });

    // In-flight: the load-bearing assertion. structureCommitStatus's
    // sibling test pins the same invariant.
    expect(useAppStore.getState().queryStatus[tabId]).toEqual({
      state: "running",
    });

    await act(async () => {
      resolveInvoke?.({
        columns: ["a"],
        rows: [["1"]],
        runtimeMs: 42,
        rowCount: 1,
      });
      await Promise.resolve();
    });

    const outcome = await outcomePromise;
    if (outcome.kind !== "completed") {
      throw new Error(`expected completed outcome, got ${outcome.kind}`);
    }
    expect(outcome.runtimeMs).toBe(42);
    expect(outcome.rowCount).toBe(1);
    expect(useAppStore.getState().queryStatus[tabId]).toBeUndefined();
  });

  it("returns failed outcome and clears running on backend rejection", async () => {
    const tabId = seedQueryTab();

    mockedInvoke.mockRejectedValueOnce(new Error("syntax error at or near"));

    let outcome!: QueryOutcome;
    await act(async () => {
      outcome = await useAppStore.getState().runQuery(tabId);
    });

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("syntax error");
    expect(useAppStore.getState().queryStatus[tabId]).toBeUndefined();
  });

  it("captures string error rejection messages", async () => {
    const tabId = seedQueryTab();
    mockedInvoke.mockRejectedValueOnce("connection lost");

    let outcome!: QueryOutcome;
    await act(async () => {
      outcome = await useAppStore.getState().runQuery(tabId);
    });

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toBe("connection lost");
  });

  it("returns noop and does not invoke when a query is already in flight", async () => {
    const tabId = seedQueryTab();

    let resolveInvoke: ((value: unknown) => void) | null = null;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve;
        }),
    );

    await act(async () => {
      void useAppStore.getState().runQuery(tabId);
    });

    expect(mockedInvoke).toHaveBeenCalledTimes(1);

    let secondOutcome!: QueryOutcome;
    await act(async () => {
      secondOutcome = await useAppStore.getState().runQuery(tabId);
    });
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(secondOutcome.kind).toBe("noop");

    await act(async () => {
      resolveInvoke?.({
        columns: [],
        rows: [],
        runtimeMs: 1,
        rowCount: 0,
      });
      await Promise.resolve();
    });
  });

  it("returns noop without invoking when the query text is empty", async () => {
    const tabId = seedQueryTab({ query: "   \n  " });
    const outcome = await useAppStore.getState().runQuery(tabId);
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(outcome.kind).toBe("noop");
    expect(useAppStore.getState().queryStatus[tabId]).toBeUndefined();
  });

  it("returns noop without invoking when the Tauri backend is unavailable", async () => {
    mockedIsTauri.mockReturnValue(false);
    const tabId = seedQueryTab();
    const outcome = await useAppStore.getState().runQuery(tabId);
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(outcome.kind).toBe("noop");
    // The pre-refactor branch wrote lastRun="Just now" to fake a run;
    // the contract is now honest — no side effects on noop.
    const tab = useAppStore
      .getState()
      .workspaceTabs.find((t) => t.id === tabId);
    expect(tab?.lastRun).toBeUndefined();
  });
});

describe("loadTablePreview status tracking", () => {
  it("transitions idle -> loading -> success", async () => {
    useAppStore.setState({ activeConnectionId: "conn-1" });

    let resolveInvoke: ((value: unknown) => void) | null = null;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve;
        }),
    );

    await act(async () => {
      void useAppStore.getState().loadTablePreview("public", "users");
    });

    const key = tableDataKey("conn-1", "public", "users");
    expect(useAppStore.getState().tableLoadStatus[key]).toEqual({
      state: "loading",
    });

    await act(async () => {
      resolveInvoke?.({
        columns: ["id"],
        rows: [["1"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 5,
      });
      await Promise.resolve();
    });

    expect(useAppStore.getState().tableLoadStatus[key].state).toBe("success");
  });

  it("captures error message on failure", async () => {
    useAppStore.setState({ activeConnectionId: "conn-1" });

    mockedInvoke.mockRejectedValueOnce(new Error("relation does not exist"));

    await act(async () => {
      await useAppStore.getState().loadTablePreview("public", "missing");
    });

    const key = tableDataKey("conn-1", "public", "missing");
    const status = useAppStore.getState().tableLoadStatus[key];
    if (status.state !== "error") {
      throw new Error(`expected error status, got ${status.state}`);
    }
    expect(status.error).toContain("relation does not exist");
  });
});

describe("store.loadTableStructure", () => {
  const baseStructure = {
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
        name: "email",
        dataType: "text",
        nullable: true,
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
    },
  };

  it("invokes load_table_structure with the connection, schema, and table", async () => {
    mockedInvoke.mockResolvedValueOnce(baseStructure);

    await useAppStore
      .getState()
      .loadTableStructure("conn-1", "public", "users");

    expect(mockedInvoke).toHaveBeenCalledWith("load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
  });

  it("populates tableStructure on success and marks status as success", async () => {
    mockedInvoke.mockResolvedValueOnce(baseStructure);

    await useAppStore
      .getState()
      .loadTableStructure("conn-1", "public", "users");

    const key = tableStructureKey("conn-1", "public", "users");
    const state = useAppStore.getState();
    expect(state.tableStructure[key]).toMatchObject({
      columns: baseStructure.columns,
      primaryKey: ["id"],
      capabilities: {
        columns: true,
        primaryKey: true,
        foreignKeys: true,
        indexes: true,
        constraints: true,
      },
    });
    expect(state.tableStructureStatus[key]).toEqual({ state: "success" });
  });

  it("preserves capability flags when the backend reports them as false", async () => {
    mockedInvoke.mockResolvedValueOnce({
      ...baseStructure,
      capabilities: {
        columns: true,
        primaryKey: true,
        foreignKeys: false,
        indexes: false,
        constraints: false,
      },
    });

    await useAppStore
      .getState()
      .loadTableStructure("conn-1", "public", "users");

    const key = tableStructureKey("conn-1", "public", "users");
    const state = useAppStore.getState();
    // The boundary normalizer fills the Plan 016 security flags a payload
    // omits; every flag the backend did report stays as reported.
    expect(state.tableStructure[key]?.capabilities).toEqual({
      columns: true,
      primaryKey: true,
      foreignKeys: false,
      indexes: false,
      constraints: false,
      triggers: false,
      policies: false,
      privileges: false,
    });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("permission denied"));

    await useAppStore
      .getState()
      .loadTableStructure("conn-1", "public", "users");

    const key = tableStructureKey("conn-1", "public", "users");
    const status = useAppStore.getState().tableStructureStatus[key];
    if (!status || status.state !== "error") {
      throw new Error(`expected error status, got ${status?.state}`);
    }
    expect(status.error).toContain("permission denied");
  });

  it("sets loading status before resolving", async () => {
    let resolveInvoke: ((value: unknown) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve as (value: unknown) => void;
        }),
    );

    const key = tableStructureKey("conn-1", "public", "users");
    const promise = useAppStore
      .getState()
      .loadTableStructure("conn-1", "public", "users");

    expect(useAppStore.getState().tableStructureStatus[key]?.state).toBe(
      "loading",
    );

    resolveInvoke?.(baseStructure);
    await promise;

    expect(useAppStore.getState().tableStructureStatus[key]?.state).toBe(
      "success",
    );
  });
});

describe("connectConnection error feedback", () => {
  it("preserves runtime lifetime ownership when stored connections reload", async () => {
    const live: Connection = {
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
      latency: "7 ms",
      ssl: false,
    };
    const { status: _status, latency: _latency, ...stored } = live;
    useAppStore.setState({
      connections: [live],
      connectionEpochs: { "conn-1": 6 },
      connectionTransitionIds: ["conn-1"],
    });
    mockedInvoke.mockResolvedValueOnce([stored]);

    await useAppStore.getState().loadConnections();

    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
    expect(useAppStore.getState().connections[0]?.latency).toBe("7 ms");
    expect(useAppStore.getState().connectionEpochs).toEqual({ "conn-1": 6 });
    expect(useAppStore.getState().connectionTransitionIds).toEqual(["conn-1"]);
  });

  it("shows a stopped managed server as starting until cold connect completes", async () => {
    const managedServer: ManagedServerWithStatus = {
      id: "server-1",
      name: "Local",
      engine: "PostgreSQL",
      version: "17",
      port: 5433,
      containerName: "dbunk-local",
      volumeName: "dbunk-local-data",
      database: "postgres",
      user: "postgres",
      connectionId: "conn-1",
      createdAt: "2026-07-27T00:00:00Z",
      status: "stopped",
      volumeExists: true,
    };
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5433,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: false,
        },
      ],
      managedServers: [managedServer],
    });

    let resolveConnect: ((value: unknown) => void) | undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveConnect = resolve;
          }),
      )
      .mockResolvedValueOnce(emptyPgObjectCatalog())
      .mockResolvedValueOnce([{ ...managedServer, status: "running" }]);

    const connect = useAppStore.getState().connectConnection("conn-1");
    expect(useAppStore.getState().managedServers[0]?.status).toBe("starting");

    resolveConnect?.({ latencyMs: 12 });
    await connect;

    expect(useAppStore.getState().managedServers[0]?.status).toBe("running");
    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
  });

  it("sets errorMessage on the connection when connect fails", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });

    mockedInvoke.mockRejectedValueOnce(
      new Error("password authentication failed"),
    );

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-1");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Disconnected");
    expect(connection?.errorMessage).toContain(
      "password authentication failed",
    );
  });

  it("clears errorMessage on successful reconnect", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
          errorMessage: "previous failure",
        },
      ],
    });

    mockedInvoke
      .mockResolvedValueOnce({ latencyMs: 10 })
      .mockResolvedValueOnce(emptyPgObjectCatalog());

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-1");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Connected");
    expect(connection?.errorMessage).toBeUndefined();
  });

  it("fetches the PostgreSQL catalog once and publishes its derived explorer", async () => {
    const catalog: PgObjectCatalog = {
      schemas: [
        {
          name: "public",
          tables: [{ name: "users" }],
          views: [{ name: "active_users" }],
          materializedViews: [],
          foreignTables: [],
          sequences: [],
          functions: [{ name: "find_user", identityArgs: "uuid" }],
          procedures: [],
          aggregates: [],
          types: [],
          domains: [],
          extensions: [],
        },
      ],
      eventTriggers: [],
      roles: [],
      tablespaces: [],
      truncated: [],
    };
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });
    mockedInvoke
      .mockResolvedValueOnce({ latencyMs: 10 })
      .mockResolvedValueOnce(catalog);

    await useAppStore.getState().connectConnection("conn-1");

    expect(mockedInvoke.mock.calls.map(([command]) => command)).toEqual([
      "connect_connection",
      "load_pg_object_catalog",
    ]);
    expect(useAppStore.getState().schemaExplorer["conn-1"]).toEqual([
      expect.objectContaining({
        name: "public",
        tables: ["users"],
        views: ["active_users"],
        functions: ["find_user(uuid)"],
      }),
    ]);
  });

  it("allows disconnect while the PostgreSQL catalog is loading", async () => {
    let resolveCatalog: (catalog: PgObjectCatalog) => void = () => undefined;
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });
    mockedInvoke
      .mockResolvedValueOnce({ latencyMs: 10 })
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveCatalog = resolve;
          }),
      )
      .mockResolvedValueOnce(undefined);

    const connect = useAppStore.getState().connectConnection("conn-1");
    await vi.waitFor(() =>
      expect(useAppStore.getState().connections[0]?.status).toBe("Connected"),
    );
    expect(useAppStore.getState().connectionTransitionIds).not.toContain(
      "conn-1",
    );

    const disconnect = useAppStore.getState().disconnectConnection("conn-1");
    await vi.waitFor(() =>
      expect(useAppStore.getState().connections[0]?.status).toBe(
        "Disconnected",
      ),
    );
    resolveCatalog(emptyPgObjectCatalog());
    await Promise.all([connect, disconnect]);

    expect(useAppStore.getState().pgObjectCatalog["conn-1"]).toEqual({
      status: "idle",
      generation: 1,
    });
    expect(useAppStore.getState().schemaExplorer["conn-1"]).toBeUndefined();
  });

  it("keeps legacy relational engines disconnected until explorer loading finishes", async () => {
    let resolveSchema: (
      schema: Array<{ name: string; tables: string[]; views: string[] }>,
    ) => void = () => undefined;
    useAppStore.setState({
      connections: [
        {
          id: "conn-mysql",
          name: "MySQL",
          database: "app",
          status: "Disconnected",
          engine: "MySQL",
          host: "localhost",
          port: 3306,
          user: "root",
          password: "",
          role: "admin",
          latency: "--",
          ssl: false,
        },
      ],
    });
    mockedInvoke.mockResolvedValueOnce({ latencyMs: 5 }).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveSchema = resolve;
        }),
    );

    const connect = useAppStore.getState().connectConnection("conn-mysql");
    await vi.waitFor(() =>
      expect(mockedInvoke).toHaveBeenCalledWith("load_schema_explorer", {
        payload: { connectionId: "conn-mysql" },
      }),
    );
    expect(useAppStore.getState().connections[0]?.status).toBe("Disconnected");
    expect(useAppStore.getState().connectionTransitionIds).toContain(
      "conn-mysql",
    );

    resolveSchema([{ name: "public", tables: ["users"], views: [] }]);
    await connect;

    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
    expect(useAppStore.getState().schemaExplorer["conn-mysql"]).toEqual([
      { name: "public", tables: ["users"], views: [] },
    ]);
  });

  it("keeps the connection connected when object catalog loading fails", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });

    mockedInvoke
      .mockResolvedValueOnce({ latencyMs: 10 })
      .mockRejectedValueOnce(new Error("permission denied"));

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-1");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Connected");
    expect(connection?.errorMessage).toBeUndefined();
    expect(useAppStore.getState().schemaExplorer["conn-1"]).toBeUndefined();
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]?.status).toBe(
      "error",
    );
  });

  it("falls back to placeholder latency when reconnect omits latency", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });

    mockedInvoke
      .mockResolvedValueOnce({})
      .mockResolvedValueOnce(emptyPgObjectCatalog());

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-1");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Connected");
    expect(connection?.latency).toBe("--");
  });

  it("skips schema-explorer fetch for keyvalue engines and stays Connected", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-redis",
          name: "Redis",
          database: "",
          status: "Disconnected",
          engine: "Redis",
          host: "localhost",
          port: 6379,
          user: "",
          password: "",
          role: "admin",
          latency: "--",
          useTls: false,
          verifyTlsCert: false,
          dbNumber: 0,
          readOnly: false,
        },
      ],
    });

    // connect_connection + redis_fetch_acl_self (Redis-only,
    // fire-and-forget). load_schema_explorer must NOT be invoked.
    mockedInvoke.mockResolvedValueOnce({ latencyMs: 3 });
    mockedInvoke.mockResolvedValueOnce({
      username: "default",
      allKeys: true,
      keyPatterns: [],
    });

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-redis");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-redis");
    expect(connection?.status).toBe("Connected");
    expect(connection?.errorMessage).toBeUndefined();
    const schemaCalls = mockedInvoke.mock.calls.filter(
      (call) => call[0] === "load_schema_explorer",
    );
    expect(schemaCalls).toHaveLength(0);
  });
});

describe("disconnectConnection cleanup", () => {
  const connectedPostgres = (id: string, name: string): Connection => ({
    id,
    name,
    database: "postgres",
    status: "Connected",
    engine: "PostgreSQL",
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "admin",
    latency: "12 ms",
    ssl: true,
  });

  it("keeps tab cleanup independent from query-session teardown", () => {
    const closeSessions = vi.fn(() => Promise.resolve());
    useAppStore.setState({
      closeQuerySessionsForConnection: closeSessions,
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select 1",
        },
      ],
      activeTabId: "tab-1",
    });

    useAppStore.getState().closeTabsForConnection("conn-1");

    expect(closeSessions).not.toHaveBeenCalled();
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
  });

  it("closes a connection's query sessions exactly once", async () => {
    const closeSessions = vi.fn(() => Promise.resolve());
    useAppStore.setState({
      closeQuerySessionsForConnection: closeSessions,
      connections: [connectedPostgres("conn-1", "Primary")],
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    expect(closeSessions).toHaveBeenCalledOnce();
    expect(closeSessions).toHaveBeenCalledWith("conn-1");
  });

  it("claims the current connection epoch after the async delete warning", async () => {
    let finishInspection: (jobs: []) => void = () => undefined;
    vi.mocked(pgToolClient.list).mockReturnValueOnce(
      new Promise<[]>((resolve) => {
        finishInspection = resolve;
      }),
    );
    let finishSessions: () => void = () => undefined;
    useAppStore.setState({
      closeQuerySessionsForConnection: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            finishSessions = resolve;
          }),
      ),
      connections: [connectedPostgres("conn-1", "Primary")],
      connectionEpochs: { "conn-1": 3 },
    });
    mockedInvoke.mockResolvedValueOnce([]);

    const deletion = useAppStore.getState().deleteConnection("conn-1");
    await vi.waitFor(() => expect(pgToolClient.list).toHaveBeenCalledOnce());
    useAppStore.setState({ connectionEpochs: { "conn-1": 8 } });
    finishInspection([]);

    await vi.waitFor(() =>
      expect(useAppStore.getState().connectionTransitionIds).toContain(
        "conn-1",
      ),
    );
    expect(useAppStore.getState().connectionEpochs["conn-1"]).toBe(9);

    finishSessions();
    await deletion;
  });

  it("refuses disconnect and delete while object DDL is applying", async () => {
    const closeSessions = vi.fn(() => Promise.resolve());
    useAppStore.setState({
      closeQuerySessionsForConnection: closeSessions,
      connections: [connectedPostgres("conn-1", "Primary")],
      pgObjectDdlApplying: { "conn-1": true },
    });

    await useAppStore.getState().disconnectConnection("conn-1");
    await useAppStore.getState().deleteConnection("conn-1");

    expect(useAppStore.getState().connections).toHaveLength(1);
    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
    expect(closeSessions).not.toHaveBeenCalled();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("marks disconnected before invalidating object metadata", async () => {
    let releaseSessions: () => void = () => undefined;
    const closeSessions = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          releaseSessions = resolve;
        }),
    );
    useAppStore.setState({
      closeQuerySessionsForConnection: closeSessions,
      connections: [connectedPostgres("conn-1", "Primary")],
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          catalog: emptyPgObjectCatalog(),
          generation: 4,
        },
      },
    });

    const disconnect = useAppStore.getState().disconnectConnection("conn-1");

    await vi.waitFor(() =>
      expect(useAppStore.getState().connections[0]?.status).toBe(
        "Disconnected",
      ),
    );
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]).toEqual({
      status: "idle",
      generation: 5,
    });

    releaseSessions();
    await disconnect;
  });

  it("rejects reconnect while disconnect teardown owns the lifetime", async () => {
    let releaseSessions: () => void = () => undefined;
    useAppStore.setState({
      closeQuerySessionsForConnection: vi.fn(
        () =>
          new Promise<void>((resolve) => {
            releaseSessions = resolve;
          }),
      ),
      connections: [connectedPostgres("conn-1", "Primary")],
    });
    mockedInvoke.mockResolvedValue(undefined);

    const disconnect = useAppStore.getState().disconnectConnection("conn-1");
    await vi.waitFor(() =>
      expect(useAppStore.getState().connectionTransitionIds).toContain(
        "conn-1",
      ),
    );
    const reconnect = useAppStore.getState().connectConnection("conn-1");

    expect(useAppStore.getState().connectionTransitionIds).toContain("conn-1");
    expect(
      mockedInvoke.mock.calls.some(
        ([command]) => command === "connect_connection",
      ),
    ).toBe(false);

    releaseSessions();
    await Promise.all([disconnect, reconnect]);

    expect(useAppStore.getState().connections[0]?.status).toBe("Disconnected");
    expect(useAppStore.getState().connectionTransitionIds).not.toContain(
      "conn-1",
    );
  });

  it("keeps a live connection when staged changes are not confirmed", async () => {
    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(false);
    useAppStore.setState({
      connections: [connectedPostgres("conn-1", "Primary")],
    });
    useAppStore.getState().openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: "conn-1",
      source: { kind: "relation", schema: "public", table: "users" },
    });
    useAppStore.getState().stageMutationDraftInsert("table:tab-1", {
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: "Ada" }],
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    expect(mockedRequestConfirm).toHaveBeenCalledOnce();
    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
    expect(useAppStore.getState().mutationDrafts["table:tab-1"]).toBeDefined();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("still disconnects locally when backend teardown fails", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("backend down"));
    useAppStore.setState({
      connections: [connectedPostgres("conn-1", "Primary")],
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select 1",
        },
      ],
      activeTabId: "tab-1",
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    const connection = useAppStore
      .getState()
      .connections.find((item) => item.id === "conn-1");
    expect(connection?.status).toBe("Disconnected");
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
  });

  it("closes query sessions once before deleting a connection", async () => {
    const closeSessions = vi.fn(() => Promise.resolve());
    mockedInvoke.mockResolvedValueOnce([]);
    useAppStore.setState({
      closeQuerySessionsForConnection: closeSessions,
      connections: [connectedPostgres("conn-1", "Primary")],
    });

    await useAppStore.getState().deleteConnection("conn-1");

    expect(closeSessions).toHaveBeenCalledOnce();
    expect(closeSessions).toHaveBeenCalledWith("conn-1");
    expect(mockedInvoke).toHaveBeenCalledWith("delete_connection", {
      payload: { connectionId: "conn-1" },
    });
  });

  it("disconnects the connection, closes its workspace tabs, and drops ephemeral workspace state", async () => {
    const conn1 = connectedPostgres("conn-1", "Primary");
    const conn2 = connectedPostgres("conn-2", "Reporting");
    const conn1DataKey = tableDataKey("conn-1", "public", "users");
    const conn2DataKey = tableDataKey("conn-2", "public", "orders");

    useAppStore.setState({
      connections: [conn1, conn2],
      activeConnectionId: "conn-1",
      activeTabId: "tab-query-1",
      workspaceTabs: [
        {
          id: "tab-query-1",
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select * from users;",
        },
        {
          id: "tab-table-1",
          kind: "table",
          label: "users",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
        },
        {
          id: "tab-query-2",
          kind: "query",
          label: "query_2.sql",
          connectionId: "conn-2",
          schema: "public",
          query: "select * from orders;",
        },
      ],
      schemaExplorer: {
        "conn-1": [{ name: "public", tables: ["users"], views: [] }],
        "conn-2": [{ name: "public", tables: ["orders"], views: [] }],
      },
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          catalog: emptyPgObjectCatalog(),
          generation: 3,
        },
        "conn-2": {
          status: "ready",
          catalog: emptyPgObjectCatalog(),
          generation: 5,
        },
      },
      pgObjectDescriptions: {
        'conn-1:["sequence","public","orders_id_seq",""]': {
          status: "loading",
          generation: 3,
        },
        'conn-2:["sequence","public","orders_id_seq",""]': {
          status: "loading",
          generation: 5,
        },
      },
      tableData: {
        [conn1DataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "name"],
          rows: [["1", "Ada"]],
          page: 1,
          pageSize: 100,
          totalRows: 1,
          runtimeMs: 4,
        },
        [conn2DataKey]: {
          connectionId: "conn-2",
          schema: "public",
          table: "orders",
          columns: ["id"],
          rows: [["42"]],
          page: 1,
          pageSize: 100,
          totalRows: 1,
          runtimeMs: 6,
        },
      },
      tableLoadStatus: {
        [conn1DataKey]: { state: "success" },
        [conn2DataKey]: { state: "success" },
      },
      databaseOverviewStats: {
        "conn-1": {
          databaseSizeBytes: 1024,
          tableSizeBytes: 512,
          indexSizeBytes: 128,
          tableCount: 1,
          schemaCount: 1,
          rowCountEstimate: 1,
          indexCount: 1,
          connectionCount: 1,
        },
        "conn-2": {
          databaseSizeBytes: 2048,
          tableSizeBytes: 1024,
          indexSizeBytes: 256,
          tableCount: 1,
          schemaCount: 1,
          rowCountEstimate: 1,
          indexCount: 1,
          connectionCount: 1,
        },
      },
      queryStatus: {
        "tab-query-1": { state: "running" },
        "tab-query-2": { state: "running" },
      },
      queryEdits: {
        "tab-query-1": { 0: { 0: "select 1" } },
        "tab-query-2": { 0: { 0: "select 2" } },
      },
      queryPreviews: {
        "tab-query-1": {
          columns: ["id"],
          rows: [["1"]],
          runtime: "4 ms",
          rowCount: "1",
          cache: "Cold",
        },
        "tab-query-2": {
          columns: ["id"],
          rows: [["42"]],
          runtime: "6 ms",
          rowCount: "1",
          cache: "Cold",
        },
      },
      tableEdits: {
        [conn1DataKey]: { 0: { 1: "Ada Lovelace" } },
        [conn2DataKey]: { 0: { 0: "42" } },
      },
      tableEditsCommitStatus: {
        [conn1DataKey]: { state: "running" },
        [conn2DataKey]: { state: "running" },
      },
      queryHistory: [
        {
          id: "history-1",
          sql: "select * from users;",
          connectionId: "conn-1",
          connectionName: "Primary",
          database: "postgres",
          engine: "PostgreSQL",
          status: "success",
          runtimeMs: 4,
          rowCount: 1,
          startedAt: "2026-05-09T12:00:00.000Z",
        },
      ],
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    const state = useAppStore.getState();
    const disconnected = state.connections.find((c) => c.id === "conn-1");
    expect(disconnected?.status).toBe("Disconnected");
    expect(disconnected?.latency).toBe("--");
    expect(state.workspaceTabs.map((tab) => tab.id)).toEqual(["tab-query-2"]);
    expect(state.activeTabId).toBe("tab-query-2");
    expect(state.activeConnectionId).toBe("conn-2");
    expect(state.schemaExplorer["conn-1"]).toBeUndefined();
    expect(state.schemaExplorer["conn-2"]).toBeDefined();
    expect(state.pgObjectCatalog["conn-1"]).toEqual({
      status: "idle",
      generation: 4,
    });
    expect(state.pgObjectCatalog["conn-2"]?.status).toBe("ready");
    expect(
      state.pgObjectDescriptions[
        'conn-1:["sequence","public","orders_id_seq",""]'
      ],
    ).toBeUndefined();
    expect(
      state.pgObjectDescriptions[
        'conn-2:["sequence","public","orders_id_seq",""]'
      ],
    ).toBeDefined();
    expect(state.tableData[conn1DataKey]).toBeUndefined();
    expect(state.tableData[conn2DataKey]).toBeDefined();
    expect(state.tableLoadStatus[conn1DataKey]).toBeUndefined();
    expect(state.databaseOverviewStats["conn-1"]).toBeUndefined();
    expect(state.queryStatus["tab-query-1"]).toBeUndefined();
    expect(state.queryStatus["tab-query-2"]).toEqual({ state: "running" });
    expect(state.queryEdits["tab-query-1"]).toBeUndefined();
    expect(state.queryPreviews["tab-query-1"]).toBeUndefined();
    expect(state.queryPreviews["tab-query-2"]).toBeDefined();
    expect(state.tableEdits[conn1DataKey]).toBeUndefined();
    expect(state.tableEdits[conn2DataKey]).toBeDefined();
    expect(state.tableEditsCommitStatus[conn1DataKey]).toBeUndefined();
    expect(state.queryHistory).toHaveLength(1);
  });
});

describe("runHealthChecks latency", () => {
  it("falls back to placeholder latency when a healthy check omits latency", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
    });

    mockedInvoke.mockResolvedValueOnce({ state: "healthy" });

    await act(async () => {
      await useAppStore.getState().runHealthChecks();
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Connected");
    expect(connection?.latency).toBe("--");
  });

  it("skips Disconnected connections so they aren't auto-connected at startup", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-pg",
          name: "PG",
          database: "postgres",
          status: "Disconnected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
        {
          id: "conn-redis",
          name: "Redis",
          database: "",
          status: "Connected",
          engine: "Redis",
          host: "localhost",
          port: 6379,
          user: "",
          password: "",
          role: "admin",
          latency: "--",
          useTls: false,
          verifyTlsCert: false,
          dbNumber: 0,
          readOnly: false,
        },
      ],
    });

    mockedInvoke.mockResolvedValueOnce({ state: "healthy", latencyMs: 4 });

    await act(async () => {
      await useAppStore.getState().runHealthChecks();
    });

    const healthCalls = mockedInvoke.mock.calls.filter(
      (call) => call[0] === "health_check_connection",
    );
    expect(healthCalls).toHaveLength(1);
    expect(healthCalls[0]?.[1]).toEqual({
      payload: { connectionId: "conn-redis" },
    });

    const pg = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-pg");
    expect(pg?.status).toBe("Disconnected");
  });

  it("invalidates PostgreSQL object metadata after a failed health check", async () => {
    const reference = {
      kind: "view" as const,
      schema: "public",
      name: "active_users",
      identityArgs: null,
    };
    useAppStore.setState({
      connections: [
        {
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
          latency: "2 ms",
          ssl: true,
        },
      ],
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          catalog: emptyPgObjectCatalog(),
          generation: 7,
        },
      },
      pgObjectDescriptions: {
        'conn-1:["view","public","active_users",""]': {
          status: "ready",
          generation: 7,
          description: {
            reference,
            owner: "postgres",
            comment: null,
            definitionSql: "CREATE VIEW public.active_users AS SELECT 1",
            facts: { kind: "view", definition: "SELECT 1" },
          },
        },
      },
    });
    mockedInvoke.mockResolvedValueOnce({
      state: "error",
      error: "connection closed",
    });

    await useAppStore.getState().runHealthChecks();

    expect(useAppStore.getState().connections[0]?.status).toBe("Disconnected");
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]).toEqual({
      status: "idle",
      generation: 8,
    });
    expect(useAppStore.getState().pgObjectDescriptions).toEqual({});
  });

  it("ignores a probe result from an earlier connection lifetime", async () => {
    let resolveHealth: (value: {
      state: "error";
      error: string;
    }) => void = () => undefined;
    useAppStore.setState({
      connections: [
        {
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
          latency: "2 ms",
          ssl: true,
        },
      ],
      connectionEpochs: { "conn-1": 4 },
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          catalog: emptyPgObjectCatalog(),
          generation: 7,
        },
      },
    });
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveHealth = resolve;
        }),
    );

    const health = useAppStore.getState().runHealthChecks();
    useAppStore.setState((state) => ({
      connectionEpochs: { ...state.connectionEpochs, "conn-1": 5 },
      connections: state.connections.map((connection) => ({
        ...connection,
        latency: "1 ms",
      })),
    }));
    resolveHealth({ state: "error", error: "old socket closed" });
    await health;

    expect(useAppStore.getState().connections[0]?.status).toBe("Connected");
    expect(useAppStore.getState().connections[0]?.latency).toBe("1 ms");
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]?.generation).toBe(
      7,
    );
  });
});

describe("runQuery overrideSql", () => {
  const getTab = (id: string) =>
    useAppStore.getState().workspaceTabs.find((t) => t.id === id);

  it("runs the tab.query when no overrideSql is supplied", async () => {
    const tabId = seedQueryTab({ query: "select * from users;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: ["id"],
        rows: [["1"]],
        runtimeMs: 12,
        rowCount: 1,
      })
      .mockResolvedValueOnce([]);

    await useAppStore.getState().runQuery(tabId);

    const runCalls = mockedInvoke.mock.calls.filter(
      (call) => call[0] === "run_query",
    );
    expect(runCalls).toHaveLength(1);
    expect(runCalls[0]?.[1]).toEqual({
      payload: { connectionId: "conn-1", query: "select * from users;" },
    });
  });

  it("runs the overrideSql when one is supplied", async () => {
    const tabId = seedQueryTab({ query: "select * from users;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 1,
        rowCount: 0,
      })
      .mockResolvedValueOnce([]);

    await useAppStore.getState().runQuery(tabId, { overrideSql: "SELECT 1" });

    expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
      payload: { connectionId: "conn-1", query: "SELECT 1" },
    });
  });

  it("falls back to tab.query when overrideSql is empty or whitespace", async () => {
    const tabId = seedQueryTab({ query: "select * from users;" });
    mockedInvoke.mockResolvedValue({
      columns: [],
      rows: [],
      runtimeMs: 0,
      rowCount: 0,
    });

    await useAppStore.getState().runQuery(tabId, { overrideSql: "" });
    await useAppStore.getState().runQuery(tabId, { overrideSql: "   \n" });

    const runCalls = mockedInvoke.mock.calls.filter(
      (call) => call[0] === "run_query",
    );
    for (const call of runCalls) {
      expect(call[1]).toEqual({
        payload: { connectionId: "conn-1", query: "select * from users;" },
      });
    }
    expect(runCalls).toHaveLength(2);
  });

  it("does not mutate tab.query when overrideSql is used", async () => {
    const tabId = seedQueryTab({ query: "select * from users;" });
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      runtimeMs: 5,
      rowCount: 0,
    });

    await useAppStore.getState().runQuery(tabId, { overrideSql: "SELECT 1" });

    expect(getTab(tabId)?.query).toBe("select * from users;");
  });

  it("records the executed SQL (override) in queryHistory", async () => {
    const tabId = seedQueryTab({ query: "select * from users;" });
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
    });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 5,
        rowCount: 0,
      })
      .mockResolvedValueOnce([]);

    await useAppStore.getState().runQuery(tabId, { overrideSql: "SELECT 1" });

    expect(useAppStore.getState().queryHistory[0]?.sql).toBe("SELECT 1");
  });

  it("stores result in queryPreviews keyed by tab ID", async () => {
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-42",
          kind: "query",
          label: "query_42.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select * from users;",
        },
      ],
    });
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id", "name"],
      rows: [["1", "Ada"]],
      runtimeMs: 17,
      rowCount: 1,
    });

    await useAppStore
      .getState()
      .runQuery("tab-42", { overrideSql: "SELECT 1" });

    const preview = useAppStore.getState().queryPreviews["tab-42"];
    expect(preview).toEqual({
      columns: ["id", "name"],
      rows: [["1", "Ada"]],
      runtime: "17 ms",
      rowCount: "1",
      cache: "Cold",
    });
  });

  it("ignores the call when tab is not a query tab", async () => {
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-table",
          kind: "table",
          label: "users",
          connectionId: "conn-1",
          schema: "public",
        },
      ],
    });

    await useAppStore
      .getState()
      .runQuery("tab-table", { overrideSql: "SELECT 1" });

    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("ignores the call when neither override nor tab.query has SQL", async () => {
    const tabId = seedQueryTab({ query: "" });

    await useAppStore.getState().runQuery(tabId, { overrideSql: "  " });

    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("query history", () => {
  const seedConnection = () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local Postgres",
          database: "postgres",
          status: "Connected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });
  };

  // Sequential assertions in a test body; cog 0.
  // fallow-ignore-next-line complexity
  it("appends a success entry to queryHistory after a successful runQuery", async () => {
    seedConnection();
    const tabId = seedQueryTab({ query: "select 1;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: ["a"],
        rows: [["1"]],
        runtimeMs: 42,
        rowCount: 1,
      })
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const entry = useAppStore.getState().queryHistory[0];
    expect(entry).toBeDefined();
    expect(entry?.sql).toBe("select 1;");
    expect(entry?.connectionId).toBe("conn-1");
    expect(entry?.connectionName).toBe("Local Postgres");
    expect(entry?.database).toBe("postgres");
    expect(entry?.engine).toBe("PostgreSQL");
    expect(entry?.status).toBe("success");
    expect(entry?.runtimeMs).toBe(42);
    expect(entry?.rowCount).toBe(1);
    expect(entry?.startedAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(entry?.id).toBeTruthy();
  });

  it("appends an error entry to queryHistory after a failed runQuery", async () => {
    seedConnection();
    const tabId = seedQueryTab({ query: "broken sql" });
    mockedInvoke
      .mockRejectedValueOnce(new Error("syntax error at or near"))
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const entry = useAppStore.getState().queryHistory[0];
    expect(entry?.status).toBe("error");
    expect(entry?.errorMessage).toContain("syntax error");
    expect(entry?.sql).toBe("broken sql");
    expect(entry?.connectionId).toBe("conn-1");
  });

  it("loadQueryHistory populates queryHistory from invoke result", async () => {
    const stored = [
      {
        id: "abc",
        sql: "select 1",
        connectionId: "conn-1",
        connectionName: "Local",
        database: "postgres",
        engine: "PostgreSQL" as const,
        status: "success" as const,
        runtimeMs: 5,
        rowCount: 1,
        startedAt: "2026-05-09T12:00:00.000Z",
      },
    ];
    mockedInvoke.mockResolvedValueOnce(stored);

    await act(async () => {
      await useAppStore.getState().loadQueryHistory();
    });

    expect(mockedInvoke.mock.calls[0]?.[0]).toBe("load_query_history");
    expect(useAppStore.getState().queryHistory).toEqual(stored);
  });

  it("does not break runQuery success when append_query_history rejects", async () => {
    seedConnection();
    const tabId = seedQueryTab({ query: "select 1;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 5,
        rowCount: 0,
      })
      .mockRejectedValueOnce(new Error("disk full"));

    let outcome!: QueryOutcome;
    await act(async () => {
      outcome = await useAppStore.getState().runQuery(tabId);
    });

    if (outcome.kind !== "completed") {
      throw new Error(`expected completed outcome, got ${outcome.kind}`);
    }
    expect(outcome.runtimeMs).toBe(5);
    // The in-memory entry should still be present even if persistence fails.
    const entry = useAppStore.getState().queryHistory[0];
    expect(entry?.sql).toBe("select 1;");
    expect(entry?.status).toBe("success");
  });

  it("reopenHistoryEntry opens a new query tab pre-filled with the SQL", async () => {
    seedConnection();
    const entry = {
      id: "abc",
      sql: "select 1 from users;",
      connectionId: "conn-1",
      connectionName: "Local Postgres",
      database: "postgres",
      engine: "PostgreSQL" as const,
      status: "success" as const,
      runtimeMs: 5,
      rowCount: 1,
      startedAt: "2026-05-09T12:00:00.000Z",
    };

    act(() => {
      useAppStore.getState().reopenHistoryEntry(entry);
    });

    const state = useAppStore.getState();
    const tab = state.workspaceTabs.find((t) => t.id === state.activeTabId);
    expect(tab?.kind).toBe("query");
    expect(tab?.query).toBe("select 1 from users;");
    expect(tab?.connectionId).toBe("conn-1");
    expect(state.activeConnectionId).toBe("conn-1");
  });

  it("reopenHistoryEntry reuses an existing tab with the same SQL+connection", async () => {
    seedConnection();
    const entry = {
      id: "abc",
      sql: "select 1 from users;",
      connectionId: "conn-1",
      connectionName: "Local Postgres",
      database: "postgres",
      engine: "PostgreSQL" as const,
      status: "success" as const,
      runtimeMs: 5,
      rowCount: 1,
      startedAt: "2026-05-09T12:00:00.000Z",
    };

    act(() => {
      useAppStore.getState().reopenHistoryEntry(entry);
    });
    const firstTabId = useAppStore.getState().activeTabId;

    act(() => {
      useAppStore.getState().reopenHistoryEntry(entry);
    });
    expect(useAppStore.getState().activeTabId).toBe(firstTabId);
    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
  });
});

const seedPostgresConnection = () => {
  const connection: Connection = {
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
  useAppStore.setState({
    connections: [connection],
    activeConnectionId: connection.id,
  });
  return connection;
};

const columnStructureChange = (
  change: ColumnChangeKind,
): Extract<StructureChange, { kind: "column" }> => ({
  kind: "column",
  change,
});

const emptyStructure = () => ({
  columns: [],
  primaryKey: null,
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
});

describe("store.pendingStructureChanges", () => {
  const key = tableStructureKey("conn-1", "public", "users");

  it("addPendingStructureChange appends a change and assigns it an id", () => {
    const change: ColumnChangeKind = { kind: "drop", columnName: "legacy" };
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange(change),
      });
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].change).toEqual({ kind: "column", change });
    expect(pending[0].schema).toBe("public");
    expect(pending[0].table).toBe("users");
    expect(typeof pending[0].id).toBe("string");
    expect(pending[0].id.length).toBeGreaterThan(0);
  });

  it("addPendingStructureChange preserves order across multiple calls", () => {
    const a: ColumnChangeKind = { kind: "drop", columnName: "a" };
    const b: ColumnChangeKind = { kind: "drop", columnName: "b" };
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange(a),
      });
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange(b),
      });
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending.map((p) => p.change)).toEqual([
      { kind: "column", change: a },
      { kind: "column", change: b },
    ]);
  });

  it("rejects mixed variants in one table's pending list", () => {
    seedPostgresConnection();
    const pgKey = tableStructureKey("conn-1", "public", "accounts");
    act(() => {
      useAppStore.getState().addPendingStructureChange(pgKey, {
        schema: "public",
        table: "accounts",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "accounts",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });

    expect(() =>
      act(() => {
        useAppStore.getState().addPendingStructureChange(pgKey, {
          schema: "public",
          table: "accounts",
          change: columnStructureChange({
            kind: "drop",
            columnName: "other_legacy",
          }),
        });
      }),
    ).toThrow(/cannot mix/i);

    const pgPending = useAppStore.getState().pendingStructureChanges[pgKey];
    expect(pgPending).toHaveLength(1);
    expect(pgPending?.every((entry) => entry.change.kind === "pg-op")).toBe(
      true,
    );
  });

  it("rejects PostgreSQL operations for non-PostgreSQL connections", () => {
    const pg = seedPostgresConnection();
    const clickHouse: Connection = {
      id: pg.id,
      name: pg.name,
      database: pg.database,
      status: pg.status,
      host: pg.host,
      port: 8123,
      user: pg.user,
      password: pg.password,
      role: pg.role,
      latency: pg.latency,
      engine: "ClickHouse",
      useHttps: false,
      urlPath: "",
    };
    useAppStore.setState({ connections: [clickHouse] });

    expect(() =>
      act(() => {
        useAppStore.getState().addPendingStructureChange(key, {
          schema: "public",
          table: "users",
          change: {
            kind: "pg-op",
            op: {
              op: "dropColumn",
              schema: "public",
              table: "users",
              name: "legacy",
              cascade: false,
            },
          },
        });
      }),
    ).toThrow(/require a PostgreSQL connection/i);
    expect(useAppStore.getState().pendingStructureChanges[key]).toBeUndefined();
  });

  it("removePendingStructureChange removes only the targeted entry", () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({ kind: "drop", columnName: "a" }),
      });
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({ kind: "drop", columnName: "b" }),
      });
    });
    const firstId =
      useAppStore.getState().pendingStructureChanges[key]?.[0]?.id ?? "";
    act(() => {
      useAppStore.getState().removePendingStructureChange(key, firstId);
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].id).not.toBe(firstId);
  });

  it("clearPendingStructureChanges drops all changes for the key", () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({ kind: "drop", columnName: "a" }),
      });
    });
    act(() => {
      useAppStore.getState().clearPendingStructureChanges(key);
    });
    expect(
      useAppStore.getState().pendingStructureChanges[key] ?? [],
    ).toHaveLength(0);
  });
});

describe("store.commitStructureChanges", () => {
  const key = tableStructureKey("conn-1", "public", "users");

  beforeEach(() => {
    seedPostgresConnection();
  });

  it("does nothing when there are no pending changes", async () => {
    const outcome = await useAppStore.getState().commitStructureChanges(key);
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(outcome.kind).toBe("noop");
  });

  it("routes a pg-op batch through apply_object_ddl and clears pending on success", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "setColumnNullable",
            schema: "public",
            table: "users",
            name: "email",
            nullable: false,
          },
        },
      });
    });
    mockedInvoke
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 9 })
      .mockResolvedValueOnce(emptyStructure());

    const versionBefore = useAppStore.getState().pgObjectDdlVersion;
    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "apply_object_ddl", {
      payload: {
        connectionId: "conn-1",
        ops: [
          {
            op: "setColumnNullable",
            schema: "public",
            table: "users",
            name: "email",
            nullable: false,
          },
        ],
        confirmed: false,
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
    const state = useAppStore.getState();
    expect(state.pendingStructureChanges[key] ?? []).toHaveLength(0);
    expect(state.structureCommitStatus[key]).toBeUndefined();
    // Other reviewed-but-unapplied DDL previews must invalidate.
    expect(state.pgObjectDdlVersion).toBe(versionBefore + 1);
    expect(outcome).toEqual({ kind: "completed", runtimeMs: 9 });
  });

  it("keeps pending, names the statement, and still refreshes after a partial pg-op failure", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });
    mockedInvoke
      .mockRejectedValueOnce({
        kind: "database",
        code: "42701",
        message: "column already exists",
        position: null,
        appliedStatements: 1,
        statementIndex: 1,
      })
      .mockResolvedValueOnce(emptyStructure());

    const outcome = await useAppStore.getState().commitStructureChanges(key, {
      statements: [{ summary: "Drop column legacy" }, { summary: "Add check" }],
    });

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("Statement 2 (Add check)");
    expect(outcome.reason).toContain("column already exists");
    expect(outcome.reason).toContain("[42701]");
    expect(outcome.reason).toContain("1 earlier statement was applied");
    // Something applied, so the structure refresh fan-out must run …
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });
    // … but the batch stays queued for edit-and-retry.
    expect(useAppStore.getState().pendingStructureChanges[key]).toHaveLength(1);
    expect(useAppStore.getState().structureCommitStatus[key]).toBeUndefined();
  });

  it("preserves ops queued while a pg-op apply is in flight", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });
    let resolveApply!: (value: unknown) => void;
    mockedInvoke
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveApply = resolve;
        }),
      )
      .mockResolvedValueOnce(emptyStructure());

    const promise = useAppStore.getState().commitStructureChanges(key);
    await Promise.resolve();
    // A second edit arrives while the apply is running.
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "renameColumn",
            schema: "public",
            table: "users",
            name: "email",
            newName: "contact_email",
          },
        },
      });
    });

    resolveApply({ appliedStatements: 1, runtimeMs: 5 });
    const outcome = await promise;

    expect(outcome).toEqual({ kind: "completed", runtimeMs: 5 });
    const remaining = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(remaining).toHaveLength(1);
    expect(remaining[0].change).toEqual({
      kind: "pg-op",
      op: {
        op: "renameColumn",
        schema: "public",
        table: "users",
        name: "email",
        newName: "contact_email",
      },
    });
  });

  it("refuses a pg-op apply while another DDL apply holds the connection lock", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });
    expect(useAppStore.getState().beginPgObjectDdlApply("conn-1")).toBe(true);

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(outcome).toEqual({
      kind: "failed",
      reason: "Another DDL apply is already running on this connection.",
    });
    expect(mockedInvoke).not.toHaveBeenCalled();
    // The foreign lock is not released by the refused commit.
    expect(useAppStore.getState().pgObjectDdlApplying["conn-1"]).toBe(true);
    useAppStore.getState().endPgObjectDdlApply("conn-1");
  });

  it("refuses a pg-op apply when the connection is not currently connected", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });
    useAppStore.setState((state) => ({
      connections: state.connections.map((connection) => ({
        ...connection,
        status: "Disconnected" as const,
      })),
    }));

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(outcome).toEqual({
      kind: "failed",
      reason: "Connect to the PostgreSQL database before applying DDL.",
    });
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(useAppStore.getState().pendingStructureChanges[key]).toHaveLength(1);
  });

  it("revalidates the table's loaded object description after a dropIndex apply", async () => {
    const tableReference = {
      kind: "table",
      schema: "public",
      name: "users",
      identityArgs: null,
    } as const;
    const descriptionKey = pgObjectDescriptionKey("conn-1", tableReference);
    const loadDescription = vi.fn(() => Promise.resolve("ready" as const));
    useAppStore.setState({
      pgObjectDescriptions: {
        [descriptionKey]: { status: "ready", generation: 0 },
      },
      loadPgObjectDescription: loadDescription,
    });
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          // dropIndex is the one op whose refresh scope cannot name the
          // table; the structure commit must supply it.
          op: {
            op: "dropIndex",
            schema: "public",
            name: "users_email_idx",
            concurrently: false,
            cascade: false,
          },
        },
      });
    });
    mockedInvoke
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 4 })
      .mockResolvedValueOnce(emptyStructure());

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(outcome).toEqual({ kind: "completed", runtimeMs: 4 });
    expect(loadDescription).toHaveBeenCalledWith("conn-1", tableReference, 0);
  });

  it.each([
    {
      name: "continues refresh after catalog stale at the same generation",
      catalogResult: "stale",
      firstDescriptionResult: "ready",
    },
    {
      name: "continues later descriptions after one is stale at the same generation",
      catalogResult: "ready",
      firstDescriptionResult: "stale",
    },
  ] as const)("$name", async ({ catalogResult, firstDescriptionResult }) => {
    const tableReference = {
      kind: "table",
      schema: "public",
      name: "users",
      identityArgs: null,
    } as const;
    const routineReference = {
      kind: "function",
      schema: "audit",
      name: "touch_users",
      identityArgs: "",
    } as const;
    const otherConnectionReference = {
      kind: "function",
      schema: "public",
      name: "unrelated",
      identityArgs: "",
    } as const;
    const loadCatalog = vi.fn(
      async (): Promise<"ready" | "stale"> => catalogResult,
    );
    let descriptionCalls = 0;
    const loadDescription = vi.fn(async (): Promise<"ready" | "stale"> => {
      const result = descriptionCalls === 0 ? firstDescriptionResult : "ready";
      descriptionCalls += 1;
      return result;
    });
    useAppStore.setState({
      pgObjectCatalog: {
        "conn-1": {
          status: "ready",
          generation: 3,
          catalog: emptyPgObjectCatalog(),
        },
      },
      pgObjectDescriptions: {
        [pgObjectDescriptionKey("conn-1", tableReference)]: {
          status: "loading",
          generation: 3,
        },
        [pgObjectDescriptionKey("conn-1", routineReference)]: {
          status: "loading",
          generation: 3,
        },
        [pgObjectDescriptionKey("conn-2", otherConnectionReference)]: {
          status: "loading",
          generation: 0,
        },
      },
      loadPgObjectCatalog: loadCatalog,
      loadPgObjectDescription: loadDescription,
    });
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "createFunction",
            schema: "audit",
            name: "touch_users",
            orReplace: true,
            arguments: "",
            returns: "trigger",
            language: "plpgsql",
            body: "BEGIN RETURN NEW; END;",
            volatility: "volatile",
            strict: false,
            securityDefiner: false,
            parallel: null,
          },
        },
      });
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "createTrigger",
            schema: "public",
            table: "users",
            name: "users_touch",
            timing: "before",
            events: [{ kind: "update", columns: [] }],
            forEach: "row",
            when: null,
            functionSchema: "audit",
            functionName: "touch_users",
            arguments: [],
            orReplace: false,
          },
        },
      });
    });
    mockedInvoke
      .mockResolvedValueOnce({ appliedStatements: 2, runtimeMs: 4 })
      .mockResolvedValueOnce(emptyStructure());

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(outcome).toEqual({ kind: "completed", runtimeMs: 4 });
    expect(loadCatalog).toHaveBeenCalledWith("conn-1", 3);
    expect(loadDescription).toHaveBeenNthCalledWith(
      1,
      "conn-1",
      tableReference,
      3,
    );
    expect(loadDescription).toHaveBeenNthCalledWith(
      2,
      "conn-1",
      routineReference,
      3,
    );
    expect(loadDescription).not.toHaveBeenCalledWith(
      "conn-2",
      otherConnectionReference,
      expect.anything(),
    );
  });

  it("does not refresh when a pg-op batch fails before anything applied", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: {
          kind: "pg-op",
          op: {
            op: "dropColumn",
            schema: "public",
            table: "users",
            name: "legacy",
            cascade: false,
          },
        },
      });
    });
    mockedInvoke.mockRejectedValueOnce({
      kind: "policyBlocked",
      reason: "Safe Mode blocks DDL on this connection.",
    });

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    expect(outcome).toEqual({
      kind: "failed",
      reason: "Safe Mode blocks DDL on this connection.",
    });
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().pendingStructureChanges[key]).toHaveLength(1);
    expect(useAppStore.getState().structureCommitStatus[key]).toBeUndefined();
  });

  it("invokes execute_ddl with generated SQL and clears pending on success", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });
    // Deferred first invoke so we can observe the in-flight `running`
    // lifecycle slot — the only state in the store while the DDL is
    // applying, and the load-bearing reason `structureCommitStatus`
    // exists at all (Commit button disabled across tab unmount).
    let resolveDdl!: (value: { runtimeMs: number }) => void;
    mockedInvoke
      .mockReturnValueOnce(
        new Promise<{ runtimeMs: number }>((resolve) => {
          resolveDdl = resolve;
        }),
      )
      .mockResolvedValueOnce({
        columns: [],
        primaryKey: null,
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
      });

    const promise = useAppStore.getState().commitStructureChanges(key);
    // Flush the synchronous `set({ running })` before observing.
    await Promise.resolve();
    expect(useAppStore.getState().structureCommitStatus[key]).toEqual({
      state: "running",
    });

    resolveDdl({ runtimeMs: 12 });
    const outcome = await promise;

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "execute_ddl", {
      payload: {
        connectionId: "conn-1",
        sql: 'ALTER TABLE "public"."users" DROP COLUMN "legacy";',
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_structure", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
      },
    });

    const state = useAppStore.getState();
    expect(state.pendingStructureChanges[key] ?? []).toHaveLength(0);
    expect(state.structureCommitStatus[key]).toBeUndefined();
    if (outcome.kind !== "completed") {
      throw new Error(`expected completed outcome, got ${outcome.kind}`);
    }
    expect(outcome.runtimeMs).toBe(12);
  });

  it("requests a descriptor refresh for post-DDL table browses", async () => {
    const refreshBrowses = vi.fn(() => Promise.resolve());
    useAppStore.setState({ refreshTableBrowsesForRelation: refreshBrowses });
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });
    mockedInvoke
      .mockResolvedValueOnce({ runtimeMs: 12 })
      .mockResolvedValueOnce({
        columns: [],
        primaryKey: null,
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
      });

    await useAppStore.getState().commitStructureChanges(key);

    expect(refreshBrowses).toHaveBeenCalledWith("conn-1", "public", "users", {
      refreshStructure: true,
      invalidateExactCount: true,
    });
  });

  it("returns failed when the connection has been removed", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });
    // Wipe the connection between adding the pending change and
    // committing — the early-validation `failed` path that bypasses
    // the running lifecycle slot.
    useAppStore.setState({ connections: [] });

    await expectCommitStructureFailure(key, /connection not found/i);
    // Pending changes survive so the user can retry after fixing the
    // connection.
    expect(useAppStore.getState().pendingStructureChanges[key]).toHaveLength(1);
  });

  it("returns failed and clears the running slot when the backend is unavailable", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });
    // Force the !isTauri() short-circuit. This is the only `failed`
    // path that exercises the running → cleared transition, so it's
    // the load-bearing test for clearLifecycle.
    mockedIsTauri.mockReturnValue(false);
    await expectCommitStructureFailure(key, /backend is unavailable/i);
  });

  it("preserves pending and surfaces error on backend failure", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });
    mockedInvoke.mockRejectedValueOnce(new Error("permission denied"));

    const outcome = await useAppStore.getState().commitStructureChanges(key);

    const state = useAppStore.getState();
    expect(state.pendingStructureChanges[key] ?? []).toHaveLength(1);
    expect(state.structureCommitStatus[key]).toBeUndefined();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toBe("permission denied");
    // Should not refresh structure on failure.
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("rejects commit when active engine is not PostgreSQL", async () => {
    useAppStore.setState((state) => ({
      // Re-emit the connection as a MySQL variant; switching engines is
      // a transform between Connection union members, not a property
      // edit. Slice 4 (#16) disables this in the actual edit dialog.
      connections: state.connections.map((c) =>
        c.id === "conn-1"
          ? ({
              id: c.id,
              name: c.name,
              database: c.database,
              status: c.status,
              host: c.host,
              port: c.port,
              user: c.user,
              password: c.password,
              role: c.role,
              latency: c.latency,
              engine: "MySQL",
              ssl: true,
            } satisfies import("@/lib/store").MySqlConnection)
          : c,
      ),
      tableStructure: {
        ...state.tableStructure,
        [key]: {
          columns: [],
          primaryKey: null,
          foreignKeys: [],
          indexes: [],
          constraints: [],
          // MySQL is in the "unsupported" tier — capabilities reflect that
          // and the frontend short-circuits without round-tripping.
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
          capabilities: {
            columns: true,
            primaryKey: false,
            foreignKeys: false,
            indexes: false,
            constraints: false,
            canInsertRows: false,
            canUpdateRows: false,
            canDeleteRows: false,
            canAlterSchema: false,
            uniquenessGuarantee: "best-effort",
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    }));
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: columnStructureChange({
          kind: "drop",
          columnName: "legacy",
        }),
      });
    });

    await expectCommitStructureFailure(key, /MySQL|does not support/i);
  });
});

describe("store.loadSchemaRelationships", () => {
  const relsResult = {
    tables: [
      { schema: "public", name: "users", columnCount: 4 },
      { schema: "public", name: "orders", columnCount: 6 },
    ],
    foreignKeys: [
      {
        constraintName: "orders_user_id_fkey",
        fromSchema: "public",
        fromTable: "orders",
        fromColumns: ["user_id"],
        toSchema: "public",
        toTable: "users",
        toColumns: ["id"],
      },
    ],
  };

  // The expanded payload: Relationship Cardinality, FK actions,
  // junction markers, and Trigger Indicator metadata per Table Card.
  const expandedRelsResult = {
    tables: [
      {
        schema: "public",
        name: "users",
        columnCount: 4,
        isJunctionTable: false,
        triggers: [
          {
            name: "users_audit",
            table: "users",
            columns: ["email"],
            timing: "BEFORE",
            events: ["UPDATE"],
            orientation: "ROW",
            enabled: true,
            functionName: "audit_users",
          },
        ],
      },
      {
        schema: "public",
        name: "user_groups",
        columnCount: 2,
        isJunctionTable: true,
      },
    ],
    foreignKeys: [
      {
        constraintName: "user_groups_user_id_fkey",
        fromSchema: "public",
        fromTable: "user_groups",
        fromColumns: ["user_id"],
        toSchema: "public",
        toTable: "users",
        toColumns: ["id"],
        relationshipType: "foreign key",
        cardinality: "one-to-many",
        cardinalityReason:
          "Referencing columns are not constrained unique on the referencing table",
        onUpdate: "NO ACTION",
        onDelete: "CASCADE",
        fkColumnsNullable: false,
        fkColumnsUnique: false,
        isJunctionParticipant: true,
      },
    ],
  };

  it("invokes load_schema_relationships with the right payload", async () => {
    mockedInvoke.mockResolvedValueOnce(relsResult);

    await useAppStore.getState().loadSchemaRelationships("conn-1", "public");

    expect(mockedInvoke).toHaveBeenCalledWith("load_schema_relationships", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
      },
    });
  });

  it("populates schemaRelationships and marks status success on resolve", async () => {
    mockedInvoke.mockResolvedValueOnce(relsResult);

    await useAppStore.getState().loadSchemaRelationships("conn-1", "public");

    const state = useAppStore.getState();
    const key = "conn-1::public";
    expect(state.schemaRelationships[key]).toEqual(relsResult);
    expect(state.schemaRelationshipsStatus[key]).toEqual({ state: "success" });
  });

  it("accepts the expanded relationship metadata payload", async () => {
    mockedInvoke.mockResolvedValueOnce(expandedRelsResult);

    await useAppStore.getState().loadSchemaRelationships("conn-1", "public");

    const stored = useAppStore.getState().schemaRelationships["conn-1::public"];
    expect(stored).toEqual(expandedRelsResult);
    expect(stored?.foreignKeys[0]?.cardinality).toBe("one-to-many");
    expect(stored?.foreignKeys[0]?.isJunctionParticipant).toBe(true);
    expect(stored?.tables[1]?.isJunctionTable).toBe(true);
    expect(stored?.tables[0]?.triggers?.[0]?.timing).toBe("BEFORE");
  });

  it("captures the error message on rejection", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("permission denied"));

    await useAppStore.getState().loadSchemaRelationships("conn-1", "public");

    const status =
      useAppStore.getState().schemaRelationshipsStatus["conn-1::public"];
    if (status?.state !== "error") {
      throw new Error(`expected error, got ${status?.state}`);
    }
    expect(status.error).toContain("permission denied");
  });

  it("transitions to loading before resolving", async () => {
    let resolveInvoke: ((value: unknown) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve as (value: unknown) => void;
        }),
    );

    const promise = useAppStore
      .getState()
      .loadSchemaRelationships("conn-1", "public");

    expect(
      useAppStore.getState().schemaRelationshipsStatus["conn-1::public"],
    ).toEqual({ state: "loading" });

    resolveInvoke?.(relsResult);
    await promise;

    expect(
      useAppStore.getState().schemaRelationshipsStatus["conn-1::public"]?.state,
    ).toBe("success");
  });

  it("is a no-op when connectionId is empty", async () => {
    await useAppStore.getState().loadSchemaRelationships("", "public");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("store schema map positions and prefs", () => {
  it("loads schema map positions into the nested cache", async () => {
    mockedInvoke.mockResolvedValueOnce([
      { tableId: "public.users", x: 10, y: 20 },
      { tableId: "public.orders", x: 30, y: 40 },
    ]);

    await useAppStore.getState().loadSchemaMapPositions("conn-1", "public");

    expect(mockedInvoke).toHaveBeenCalledWith("load_schema_map_positions", {
      payload: { connectionId: "conn-1", schema: "public" },
    });
    expect(useAppStore.getState().schemaMapPositions["conn-1"].public).toEqual({
      "public.users": { x: 10, y: 20 },
      "public.orders": { x: 30, y: 40 },
    });
    expect(
      useAppStore.getState().schemaMapPositionsStatus["conn-1"].public,
    ).toEqual({ state: "success" });
  });

  it("records schema map position load errors", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("disk full"));

    await useAppStore.getState().loadSchemaMapPositions("conn-1", "public");

    expect(
      useAppStore.getState().schemaMapPositionsStatus["conn-1"].public,
    ).toEqual({ state: "error", error: "disk full" });
  });

  it("saveSchemaMapPosition writes optimistically and flushes to Tauri", async () => {
    mockedInvoke.mockResolvedValueOnce(undefined);

    await useAppStore
      .getState()
      .saveSchemaMapPosition("conn-1", "public", "public.users", 50, 60);

    expect(useAppStore.getState().schemaMapPositions["conn-1"].public).toEqual({
      "public.users": { x: 50, y: 60 },
    });
    expect(mockedInvoke).toHaveBeenCalledWith("save_schema_map_position", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        tableId: "public.users",
        x: 50,
        y: 60,
      },
    });
  });

  it("resetSchemaMapPositions clears local overrides and calls Tauri", async () => {
    mockedInvoke.mockResolvedValueOnce(undefined);
    useAppStore.setState({
      schemaMapPositions: {
        "conn-1": { public: { "public.users": { x: 10, y: 20 } } },
      },
    });

    await useAppStore.getState().resetSchemaMapPositions("conn-1", "public");

    expect(useAppStore.getState().schemaMapPositions["conn-1"].public).toEqual(
      {},
    );
    expect(mockedInvoke).toHaveBeenCalledWith("reset_schema_map_positions", {
      payload: { connectionId: "conn-1", schema: "public" },
    });
  });

  it("loads schema map prefs", async () => {
    mockedInvoke.mockResolvedValueOnce({
      routing: "step",
      attrMode: "keys-only",
      showTypes: false,
      showNulls: true,
      showComments: true,
    });

    await useAppStore.getState().loadSchemaMapPrefs("conn-1", "public");

    expect(mockedInvoke).toHaveBeenCalledWith("load_schema_map_prefs", {
      payload: { connectionId: "conn-1", schema: "public" },
    });
    expect(useAppStore.getState().schemaMapPrefs["conn-1"].public).toEqual({
      routing: "step",
      attrMode: "keys-only",
      showTypes: false,
      showNulls: true,
      showComments: true,
    });
  });

  it("setSchemaMapPref updates locally and persists the patch", async () => {
    mockedInvoke.mockResolvedValueOnce({
      routing: "bezier",
      attrMode: "all",
      showTypes: true,
      showNulls: true,
      showComments: false,
    });

    await useAppStore
      .getState()
      .setSchemaMapPref("conn-1", "public", { showNulls: true });

    expect(mockedInvoke).toHaveBeenCalledWith("save_schema_map_prefs", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        patch: { showNulls: true },
      },
    });
    expect(useAppStore.getState().schemaMapPrefs["conn-1"].public).toEqual({
      routing: "bezier",
      attrMode: "all",
      showTypes: true,
      showNulls: true,
      showComments: false,
    });
  });

  it("schema map loaders no-op on empty ids", async () => {
    await useAppStore.getState().loadSchemaMapPositions("", "public");
    await useAppStore.getState().loadSchemaMapPrefs("conn-1", "");
    await useAppStore
      .getState()
      .saveSchemaMapPosition("conn-1", "public", "", 1, 2);

    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("store.loadDatabaseOverviewStats", () => {
  const statsResult = {
    databaseSizeBytes: 10485760,
    tableSizeBytes: 4194304,
    indexSizeBytes: 2097152,
  };

  it("invokes load_database_overview_stats with the right payload", async () => {
    mockedInvoke.mockResolvedValueOnce(statsResult);

    await useAppStore.getState().loadDatabaseOverviewStats("conn-1");

    expect(mockedInvoke).toHaveBeenCalledWith("load_database_overview_stats", {
      payload: {
        connectionId: "conn-1",
      },
    });
  });

  it("populates databaseOverviewStats and marks status success on resolve", async () => {
    mockedInvoke.mockResolvedValueOnce(statsResult);

    await useAppStore.getState().loadDatabaseOverviewStats("conn-1");

    const state = useAppStore.getState();
    expect(state.databaseOverviewStats["conn-1"]).toEqual(statsResult);
    expect(state.databaseOverviewStatsStatus["conn-1"]).toEqual({
      state: "success",
    });
  });

  it("captures the error message on rejection", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("permission denied"));

    await useAppStore.getState().loadDatabaseOverviewStats("conn-1");

    const status = useAppStore.getState().databaseOverviewStatsStatus["conn-1"];
    if (status?.state !== "error") {
      throw new Error(`expected error, got ${status?.state}`);
    }
    expect(status.error).toContain("permission denied");
  });

  it("does nothing without a connection id", async () => {
    await useAppStore.getState().loadDatabaseOverviewStats("");

    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("store.focusTableInSchemaMap", () => {
  it("opens a table tab via the existing openTableTab path", async () => {
    mockedInvoke.mockResolvedValue({
      columns: [],
      rows: [],
      page: 1,
      pageSize: 100,
      totalRows: 0,
      runtimeMs: 0,
    });
    useAppStore.setState({ activeConnectionId: "conn-1" });

    useAppStore.getState().focusTableInSchemaMap("conn-1", "public", "orders");

    const tabs = useAppStore.getState().workspaceTabs;
    const opened = tabs.find(
      (tab) => tab.kind === "table" && tab.table === "orders",
    );
    expect(opened).toBeDefined();
    expect(opened?.schema).toBe("public");
    expect(opened?.connectionId).toBe("conn-1");
    expect(useAppStore.getState().activeTabId).toBe(opened?.id);
  });

  it("focuses an existing table tab without re-opening", () => {
    useAppStore.setState({
      activeConnectionId: "conn-1",
      workspaceTabs: [
        {
          id: "tab-existing",
          kind: "table",
          label: "orders",
          connectionId: "conn-1",
          schema: "public",
          table: "orders",
        },
      ],
    });

    useAppStore.getState().focusTableInSchemaMap("conn-1", "public", "orders");

    const tabs = useAppStore.getState().workspaceTabs;
    expect(tabs).toHaveLength(1);
    expect(useAppStore.getState().activeTabId).toBe("tab-existing");
  });
});

describe("store.commitTableEdits", () => {
  const dataKey = tableDataKey("conn-1", "public", "users");
  const structureKey = tableStructureKey("conn-1", "public", "users");

  const seedTable = ({
    primaryKey = ["id"] as string[] | null,
    indexes = [] as Array<{
      name: string;
      columns: string[];
      isUnique: boolean;
      isPrimary: boolean;
      method: string | null;
    }>,
  } = {}) => {
    seedPostgresConnection();
    useAppStore.setState({
      tableData: {
        [dataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "email", "name"],
          rows: [
            ["1", "ada@example.com", "Ada"],
            ["2", "grace@example.com", "Grace"],
          ],
          page: 1,
          pageSize: 100,
          totalRows: 2,
          runtimeMs: 1,
        },
      },
      tableStructure: {
        [structureKey]: {
          columns: [
            {
              name: "id",
              dataType: "integer",
              nullable: false,
              defaultValue: null,
              isPrimaryKey: primaryKey?.includes("id") ?? false,
              ordinalPosition: 1,
            },
            {
              name: "email",
              dataType: "text",
              nullable: false,
              defaultValue: null,
              isPrimaryKey: false,
              ordinalPosition: 2,
            },
            {
              name: "name",
              dataType: "text",
              nullable: true,
              defaultValue: null,
              isPrimaryKey: false,
              ordinalPosition: 3,
            },
          ],
          primaryKey,
          foreignKeys: [],
          indexes,
          constraints: [],
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
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
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    });
  };

  it("does nothing when there are no pending edits", async () => {
    seedTable();
    const outcome = await useAppStore.getState().commitTableEdits("users");
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(outcome.kind).toBe("noop");
  });

  it("invokes commit_cell_edits with identity + set values for each edited row", async () => {
    seedTable();
    act(() => {
      // Edit the email of row 0 and the name of row 1.
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
      useAppStore.getState().setTableEdit("users", 1, 2, "Grace H.");
    });
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 2, runtimeMs: 7 })
      // refreshTableData -> load_table_data
      .mockResolvedValueOnce({
        columns: ["id", "email", "name"],
        rows: [
          ["1", "ada@new.com", "Ada"],
          ["2", "grace@example.com", "Grace H."],
        ],
        page: 1,
        pageSize: 100,
        totalRows: 2,
        runtimeMs: 1,
      });

    await useAppStore.getState().commitTableEdits("users");

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "id", value: "1" }],
            set: [{ column: "email", value: "ada@new.com" }],
          },
          {
            rowIndex: 1,
            identity: [{ column: "id", value: "2" }],
            set: [{ column: "name", value: "Grace H." }],
          },
        ],
      },
    });
  });

  it("clears edits, refreshes data, and reports success on commit success", async () => {
    seedTable();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        columns: ["id", "email", "name"],
        rows: [["1", "ada@new.com", "Ada"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    const outcome = await useAppStore.getState().commitTableEdits("users");

    const state = useAppStore.getState();
    expect(state.tableEdits[dataKey]).toBeUndefined();
    expect(outcome.kind).toBe("completed");
    // Two invokes: commit + reload.
    expect(mockedInvoke).toHaveBeenCalledTimes(2);
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 1,
        pageSize: 100,
      },
    });
  });

  it("keeps Cell Edit state isolated by Table Session key", async () => {
    seedTable();
    const archiveDataKey = tableDataKey("conn-1", "archive", "users");
    const archiveStructureKey = tableStructureKey("conn-1", "archive", "users");
    const baseData = useAppStore.getState().tableData[dataKey];
    const baseStructure = useAppStore.getState().tableStructure[structureKey];
    if (!baseData || !baseStructure) {
      throw new Error("seedTable did not populate table session state");
    }
    useAppStore.setState((state) => ({
      tableData: {
        ...state.tableData,
        [archiveDataKey]: {
          ...baseData,
          schema: "archive",
          rows: [["99", "archived@example.com", "Archived"]],
          totalRows: 1,
        },
      },
      tableStructure: {
        ...state.tableStructure,
        [archiveStructureKey]: baseStructure,
      },
    }));

    act(() => {
      useAppStore
        .getState()
        .setTableCellEdit(
          { connectionId: "conn-1", schema: "public", table: "users" },
          0,
          1,
          "ada@new.com",
        );
      useAppStore
        .getState()
        .setTableCellEdit(
          { connectionId: "conn-1", schema: "archive", table: "users" },
          0,
          1,
          "archived@new.com",
        );
    });
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        columns: ["id", "email", "name"],
        rows: [["1", "ada@new.com", "Ada"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    const outcome = await useAppStore.getState().commitTableCellEdits({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
    });

    expect(outcome.kind).toBe("completed");
    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "id", value: "1" }],
            set: [{ column: "email", value: "ada@new.com" }],
          },
        ],
      },
    });
    const state = useAppStore.getState();
    expect(state.tableEdits[dataKey]).toBeUndefined();
    expect(state.tableEdits[archiveDataKey]?.[0]?.[1]).toBe("archived@new.com");
  });

  it("rejects legacy table-name edits when the Table Session is ambiguous", async () => {
    seedTable();
    const archiveDataKey = tableDataKey("conn-1", "archive", "users");
    const archiveStructureKey = tableStructureKey("conn-1", "archive", "users");
    const baseData = useAppStore.getState().tableData[dataKey];
    const baseStructure = useAppStore.getState().tableStructure[structureKey];
    if (!baseData || !baseStructure) {
      throw new Error("seedTable did not populate table session state");
    }
    useAppStore.setState((state) => ({
      tableData: {
        ...state.tableData,
        [archiveDataKey]: {
          ...baseData,
          schema: "archive",
        },
      },
      tableStructure: {
        ...state.tableStructure,
        [archiveStructureKey]: baseStructure,
      },
    }));
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    const outcome = await useAppStore.getState().commitTableEdits("users");

    expect(useAppStore.getState().tableEdits[dataKey]).toBeUndefined();
    expect(useAppStore.getState().tableEdits[archiveDataKey]).toBeUndefined();
    expect(warnSpy).toHaveBeenCalledWith(expect.stringMatching(/ambiguous/i));
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/ambiguous/i);
    warnSpy.mockRestore();
  });

  it("preserves edits and reports an error on commit failure", async () => {
    seedTable();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    mockedInvoke.mockRejectedValueOnce(new Error("row not found: id=1"));

    const outcome = await useAppStore.getState().commitTableEdits("users");

    const state = useAppStore.getState();
    expect(state.tableEdits[dataKey]?.[0]?.[1]).toBe("ada@new.com");
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("row not found");
    // No refresh on failure.
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("falls back to a unique non-null index when no primary key is present", async () => {
    seedTable({
      primaryKey: null,
      indexes: [
        {
          name: "users_email_key",
          columns: ["email"],
          isUnique: true,
          isPrimary: false,
          method: "btree",
        },
      ],
    });
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 2, "Ada Lovelace");
    });
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 5 })
      .mockResolvedValueOnce({
        columns: ["id", "email", "name"],
        rows: [["1", "ada@example.com", "Ada Lovelace"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    await useAppStore.getState().commitTableEdits("users");

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "commit_cell_edits", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        edits: [
          {
            rowIndex: 0,
            identity: [{ column: "email", value: "ada@example.com" }],
            set: [{ column: "name", value: "Ada Lovelace" }],
          },
        ],
      },
    });
  });

  it("does not invoke commit and reports a read-only error when no identity is available", async () => {
    seedTable({ primaryKey: null, indexes: [] });
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });

    const outcome = await useAppStore.getState().commitTableEdits("users");

    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/read.?only|primary key|unique/i);
    // Edits are kept so the user can still discard explicitly.
    expect(useAppStore.getState().tableEdits[dataKey]?.[0]?.[1]).toBe(
      "ada@new.com",
    );
  });

  it("strips edits that match the original value (no-op edits) before committing", async () => {
    seedTable();
    act(() => {
      // Set then revert to original — should be filtered out.
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@example.com");
      // A genuine edit on another row.
      useAppStore.getState().setTableEdit("users", 1, 2, "Grace H.");
    });
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 3 })
      .mockResolvedValueOnce({
        columns: ["id", "email", "name"],
        rows: [
          ["1", "ada@example.com", "Ada"],
          ["2", "grace@example.com", "Grace H."],
        ],
        page: 1,
        pageSize: 100,
        totalRows: 2,
        runtimeMs: 1,
      });

    await useAppStore.getState().commitTableEdits("users");

    const call = mockedInvoke.mock.calls[0];
    expect(call?.[0]).toBe("commit_cell_edits");
    const [, payloadArg] = call as [string, { payload: { edits: unknown[] } }];
    const payload = payloadArg.payload;
    expect(payload.edits).toHaveLength(1);
    expect(payload.edits[0]).toMatchObject({
      rowIndex: 1,
      identity: [{ column: "id", value: "2" }],
      set: [{ column: "name", value: "Grace H." }],
    });
  });
});

describe("store.addTableRow", () => {
  const dataKey = tableDataKey("conn-1", "public", "users");
  const structureKey = tableStructureKey("conn-1", "public", "users");

  const seedTableForInsert = () => {
    seedPostgresConnection();
    useAppStore.setState({
      tableData: {
        [dataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "email"],
          rows: [["1", "ada@example.com"]],
          page: 1,
          pageSize: 100,
          totalRows: 1,
          runtimeMs: 1,
        },
      },
      tableStructure: {
        [structureKey]: {
          columns: [],
          primaryKey: ["id"],
          foreignKeys: [],
          indexes: [],
          constraints: [],
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
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
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    });
  };

  it("invokes insert_row with the right payload, refreshes data, and reports success", async () => {
    seedTableForInsert();
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 1, runtimeMs: 4 })
      // refresh
      .mockResolvedValueOnce({
        columns: ["id", "email"],
        rows: [
          ["1", "ada@example.com"],
          ["2", "grace@example.com"],
        ],
        page: 1,
        pageSize: 100,
        totalRows: 2,
        runtimeMs: 1,
      });

    const outcome = await useAppStore
      .getState()
      .addTableRow("users", [{ column: "email", value: "grace@example.com" }]);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "insert_row", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        values: [{ column: "email", value: "grace@example.com" }],
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 1,
        pageSize: 100,
      },
    });
    if (outcome.kind !== "completed") {
      throw new Error(`expected completed outcome, got ${outcome.kind}`);
    }
    expect(outcome.rowsAffected).toBe(1);
  });

  it("reports an error and does not refresh when insert fails", async () => {
    seedTableForInsert();
    mockedInvoke.mockRejectedValueOnce(
      new Error('null value in column "email" violates not-null constraint'),
    );

    const outcome = await useAppStore
      .getState()
      .addTableRow("users", [{ column: "email", value: null }]);

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("not-null");
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("errors immediately when the connection is not Postgres", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "dbunk",
          status: "Connected",
          engine: "MySQL",
          host: "localhost",
          port: 3306,
          user: "root",
          password: "",
          role: "admin",
          latency: "10 ms",
          ssl: true,
        },
      ],
      activeConnectionId: "conn-1",
      tableData: {
        [dataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "email"],
          rows: [],
          page: 1,
          pageSize: 100,
          totalRows: 0,
          runtimeMs: 1,
        },
      },
      tableStructure: {
        [structureKey]: {
          columns: [],
          primaryKey: ["id"],
          foreignKeys: [],
          indexes: [],
          constraints: [],
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
          capabilities: {
            columns: true,
            primaryKey: false,
            foreignKeys: false,
            indexes: false,
            constraints: false,
            canInsertRows: false,
            canUpdateRows: false,
            canDeleteRows: false,
            canAlterSchema: false,
            uniquenessGuarantee: "best-effort",
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    });

    const outcome = await useAppStore
      .getState()
      .addTableRow("users", [{ column: "email", value: "x@y.z" }]);

    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/MySQL|does not support/i);
  });

  it("errors when there are no values to insert", async () => {
    seedTableForInsert();
    const outcome = await useAppStore.getState().addTableRow("users", []);
    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/no values|at least one/i);
  });

  it("rejects legacy table-name inserts when the Table Session is ambiguous", async () => {
    seedTableForInsert();
    const archiveDataKey = tableDataKey("conn-1", "archive", "users");
    const baseData = useAppStore.getState().tableData[dataKey];
    if (!baseData) {
      throw new Error("seedTableForInsert did not populate table data");
    }
    useAppStore.setState((state) => ({
      tableData: {
        ...state.tableData,
        [archiveDataKey]: {
          ...baseData,
          schema: "archive",
        },
      },
    }));

    const outcome = await useAppStore
      .getState()
      .addTableRow("users", [{ column: "email", value: "grace@example.com" }]);

    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/ambiguous/i);
  });
});

describe("store.deleteSelectedTableRows", () => {
  const dataKey = tableDataKey("conn-1", "public", "users");
  const structureKey = tableStructureKey("conn-1", "public", "users");

  const seedTableForDelete = ({
    primaryKey = ["id"] as string[] | null,
  } = {}) => {
    seedPostgresConnection();
    useAppStore.setState({
      tableData: {
        [dataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "email"],
          rows: [
            ["1", "ada@example.com"],
            ["2", "grace@example.com"],
            ["3", "edsger@example.com"],
          ],
          page: 1,
          pageSize: 100,
          totalRows: 3,
          runtimeMs: 1,
        },
      },
      tableStructure: {
        [structureKey]: {
          columns: [
            {
              name: "id",
              dataType: "integer",
              nullable: false,
              defaultValue: null,
              isPrimaryKey: primaryKey?.includes("id") ?? false,
              ordinalPosition: 1,
            },
            {
              name: "email",
              dataType: "text",
              nullable: false,
              defaultValue: null,
              isPrimaryKey: false,
              ordinalPosition: 2,
            },
          ],
          primaryKey,
          foreignKeys: [],
          indexes: [],
          constraints: [],
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
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
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    });
  };

  it("builds identity payloads from tableData rows and refreshes on success", async () => {
    seedTableForDelete();
    mockedInvoke
      .mockResolvedValueOnce({ rowsAffected: 2, runtimeMs: 6 })
      .mockResolvedValueOnce({
        columns: ["id", "email"],
        rows: [["3", "edsger@example.com"]],
        page: 1,
        pageSize: 100,
        totalRows: 1,
        runtimeMs: 1,
      });

    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", [0, 1]);

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "delete_rows", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        rows: [[{ column: "id", value: "1" }], [{ column: "id", value: "2" }]],
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_table_data", {
      payload: {
        connectionId: "conn-1",
        schema: "public",
        table: "users",
        page: 1,
        pageSize: 100,
      },
    });
    if (outcome.kind !== "completed") {
      throw new Error(`expected completed outcome, got ${outcome.kind}`);
    }
    expect(outcome.rowsAffected).toBe(2);
  });

  it("does not invoke and reports a read-only error when no identity is available", async () => {
    seedTableForDelete({ primaryKey: null });
    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", [0]);
    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/read.?only|primary key|unique/i);
  });

  it("does nothing when the row index list is empty", async () => {
    seedTableForDelete();
    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", []);
    expect(mockedInvoke).not.toHaveBeenCalled();
    expect(outcome.kind).toBe("noop");
  });

  it("errors immediately when the connection is not Postgres", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "dbunk",
          status: "Connected",
          engine: "MySQL",
          host: "localhost",
          port: 3306,
          user: "root",
          password: "",
          role: "admin",
          latency: "10 ms",
          ssl: true,
        },
      ],
      activeConnectionId: "conn-1",
      tableData: {
        [dataKey]: {
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          columns: ["id", "email"],
          rows: [["1", "ada@example.com"]],
          page: 1,
          pageSize: 100,
          totalRows: 1,
          runtimeMs: 1,
        },
      },
      tableStructure: {
        [structureKey]: {
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
              name: "email",
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
          triggers: [],
          policies: [],
          privileges: [],
          rowSecurity: null,
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
            triggers: false,
            policies: false,
            privileges: false,
          },
        },
      },
    });

    // Override the capability flags to reflect MySQL's "unsupported" tier
    // (the test seeded the structure as if it were a PG fixture before the
    // capability flags landed).
    useAppStore.setState((s) => ({
      tableStructure: {
        ...s.tableStructure,
        [structureKey]: {
          ...s.tableStructure[structureKey],
          capabilities: {
            ...s.tableStructure[structureKey].capabilities,
            canInsertRows: false,
            canUpdateRows: false,
            canDeleteRows: false,
            canAlterSchema: false,
          },
        },
      },
    }));

    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", [0]);

    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/MySQL|does not support/i);
  });

  it("preserves data and reports an error when the delete fails", async () => {
    seedTableForDelete();
    mockedInvoke.mockRejectedValueOnce(new Error("row not found: id=1"));

    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", [0]);

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("row not found");
    // No refresh on failure.
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("rejects legacy table-name deletes when the Table Session is ambiguous", async () => {
    seedTableForDelete();
    const archiveDataKey = tableDataKey("conn-1", "archive", "users");
    const baseData = useAppStore.getState().tableData[dataKey];
    if (!baseData) {
      throw new Error("seedTableForDelete did not populate table data");
    }
    useAppStore.setState((state) => ({
      tableData: {
        ...state.tableData,
        [archiveDataKey]: {
          ...baseData,
          schema: "archive",
        },
      },
    }));

    const outcome = await useAppStore
      .getState()
      .deleteSelectedTableRows("users", [0]);

    expect(mockedInvoke).not.toHaveBeenCalled();
    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toMatch(/ambiguous/i);
  });
});

describe("runQuery branch coverage", () => {
  it("returns noop when the tabId does not match any workspace tab", async () => {
    useAppStore.setState({ workspaceTabs: [] });
    const outcome = await useAppStore.getState().runQuery("missing-tab");
    expect(outcome.kind).toBe("noop");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("records empty connectionName/database when the tab's connection is missing", async () => {
    const tabId = seedQueryTab({ query: "select 1;" });
    // No connection seeded under conn-1, so connectionAtRun is undefined.
    useAppStore.setState({ connections: [] });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 3,
        rowCount: 0,
      })
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const entry = useAppStore.getState().queryHistory[0];
    expect(entry).toBeDefined();
    expect(entry?.connectionName).toBe("");
    expect(entry?.database).toBe("");
    // Defaults to PostgreSQL when no connection found.
    expect(entry?.engine).toBe("PostgreSQL");
  });

  it("captures non-Error rejections (string thrown) in the failure entry", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
    });
    const tabId = seedQueryTab({ query: "select 1;" });
    mockedInvoke
      .mockRejectedValueOnce("network unreachable")
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const entry = useAppStore.getState().queryHistory[0];
    expect(entry?.status).toBe("error");
    expect(entry?.errorMessage).toBe("network unreachable");
  });

  it("does not break runQuery failure when append_query_history rejects", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
    });
    const tabId = seedQueryTab({ query: "broken;" });
    mockedInvoke
      .mockRejectedValueOnce(new Error("syntax error"))
      .mockRejectedValueOnce(new Error("disk full"));

    let outcome!: QueryOutcome;
    await act(async () => {
      outcome = await useAppStore.getState().runQuery(tabId);
    });

    if (outcome.kind !== "failed") {
      throw new Error(`expected failed outcome, got ${outcome.kind}`);
    }
    expect(outcome.reason).toContain("syntax error");
    // The in-memory entry should still be present even if persistence fails.
    expect(useAppStore.getState().queryHistory[0]?.status).toBe("error");
  });

  it("caps queryHistory at 2000 entries when appending", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
      queryHistory: Array.from({ length: 2000 }, (_, idx) => ({
        id: `seed-${idx}`,
        sql: `select ${idx};`,
        connectionId: "conn-1",
        connectionName: "Local",
        database: "postgres",
        engine: "PostgreSQL" as const,
        status: "success" as const,
        runtimeMs: 1,
        rowCount: 0,
        startedAt: "2026-05-09T12:00:00.000Z",
      })),
    });
    const tabId = seedQueryTab({ query: "select fresh;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 7,
        rowCount: 0,
      })
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const history = useAppStore.getState().queryHistory;
    expect(history).toHaveLength(2000);
    expect(history[0]?.sql).toBe("select fresh;");
    // Oldest seed should have been dropped.
    expect(history.find((h) => h.id === "seed-1999")).toBeUndefined();
  });

  it("bumps the connection's lastActivityAt on a successful run", async () => {
    useAppStore.setState({
      connections: [
        {
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
          latency: "--",
          ssl: true,
        },
      ],
    });
    const tabId = seedQueryTab({ query: "select 1;" });
    mockedInvoke
      .mockResolvedValueOnce({
        columns: [],
        rows: [],
        runtimeMs: 3,
        rowCount: 0,
      })
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const conn = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(conn?.lastActivityAt).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });
});

describe("store.loadRelationStats", () => {
  it("invokes load_relation_stats and stores the result on success", async () => {
    mockedInvoke.mockResolvedValueOnce([
      {
        schema: "public",
        name: "users",
        kind: "table",
        rowCountEstimate: 1024,
        totalSizeBytes: 8192,
      },
    ]);

    await useAppStore.getState().loadRelationStats("conn-1");

    expect(mockedInvoke).toHaveBeenCalledWith("load_relation_stats", {
      payload: { connectionId: "conn-1" },
    });
    expect(useAppStore.getState().relationStats["conn-1"]).toHaveLength(1);
    expect(useAppStore.getState().relationStatsStatus["conn-1"]).toEqual({
      state: "success",
    });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("connect refused"));

    await useAppStore.getState().loadRelationStats("conn-1");

    expect(useAppStore.getState().relationStats["conn-1"]).toBeUndefined();
    expect(useAppStore.getState().relationStatsStatus["conn-1"]).toEqual({
      state: "error",
      error: "connect refused",
    });
  });

  it("no-ops when connectionId is empty", async () => {
    await useAppStore.getState().loadRelationStats("");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("store.loadServerDetails", () => {
  it("invokes load_server_details and stores the result on success", async () => {
    mockedInvoke.mockResolvedValueOnce({
      serverVersion: "PostgreSQL 16.2",
      encoding: "UTF8",
      locale: "en_US.UTF-8",
      timezone: "UTC",
      settings: [
        {
          name: "max_connections",
          setting: "100",
          unit: null,
          category: "Connections",
          shortDesc: "Sets the maximum number of concurrent connections.",
          source: "configuration file",
          bootVal: "100",
          resetVal: "100",
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
    });

    await useAppStore.getState().loadServerDetails("conn-1");

    expect(mockedInvoke).toHaveBeenCalledWith("load_server_details", {
      payload: { connectionId: "conn-1" },
    });
    expect(useAppStore.getState().serverDetails["conn-1"]?.encoding).toBe(
      "UTF8",
    );
    expect(useAppStore.getState().serverDetailsStatus["conn-1"]).toEqual({
      state: "success",
    });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("connect refused"));

    await useAppStore.getState().loadServerDetails("conn-1");

    expect(useAppStore.getState().serverDetails["conn-1"]).toBeUndefined();
    expect(useAppStore.getState().serverDetailsStatus["conn-1"]).toEqual({
      state: "error",
      error: "connect refused",
    });
  });

  it("no-ops when connectionId is empty", async () => {
    await useAppStore.getState().loadServerDetails("");
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});

describe("disconnectConnection drops relationStats caches", () => {
  it("drops relationStats and relationStatsStatus for the disconnected connection", async () => {
    useAppStore.setState({
      connections: [
        {
          id: "conn-1",
          name: "Primary",
          database: "postgres",
          status: "Connected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "",
          latency: "12 ms",
          ssl: true,
        },
      ],
      relationStats: {
        "conn-1": [
          {
            schema: "public",
            name: "users",
            kind: "table",
            rowCountEstimate: 1,
            totalSizeBytes: 1,
          },
        ],
      },
      relationStatsStatus: {
        "conn-1": { state: "success" },
      },
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    expect(useAppStore.getState().relationStats["conn-1"]).toBeUndefined();
    expect(
      useAppStore.getState().relationStatsStatus["conn-1"],
    ).toBeUndefined();
  });
});

describe("connectionOverviewTab", () => {
  const connectedPostgres = (id: string): Connection => ({
    id,
    name: id,
    database: "postgres",
    status: "Connected",
    engine: "PostgreSQL",
    host: "localhost",
    port: 5432,
    user: "postgres",
    password: "",
    role: "",
    latency: "12 ms",
    ssl: true,
  });

  it("setConnectionOverviewTab stores the active sub-tab per connection", () => {
    useAppStore.setState({
      connections: [connectedPostgres("conn-1"), connectedPostgres("conn-2")],
      connectionOverviewTab: {},
    });

    useAppStore.getState().setConnectionOverviewTab("conn-1", "tables");
    useAppStore.getState().setConnectionOverviewTab("conn-2", "details");

    expect(useAppStore.getState().connectionOverviewTab).toEqual({
      "conn-1": "tables",
      "conn-2": "details",
    });
  });

  it("setConnectionOverviewTab is a no-op when connectionId is empty", () => {
    useAppStore.setState({ connectionOverviewTab: { "conn-1": "tables" } });

    useAppStore.getState().setConnectionOverviewTab("", "schemas");

    expect(useAppStore.getState().connectionOverviewTab).toEqual({
      "conn-1": "tables",
    });
  });

  it("disconnectConnection drops the connection's sub-tab entry", async () => {
    useAppStore.setState({
      connections: [connectedPostgres("conn-1"), connectedPostgres("conn-2")],
      connectionOverviewTab: {
        "conn-1": "schemas",
        "conn-2": "details",
      },
      connectionSchemaMapSchema: {
        "conn-1": "public",
        "conn-2": "audit",
      },
      schemaMapPositions: {
        "conn-1": { public: { "public.users": { x: 1, y: 2 } } },
        "conn-2": { audit: { "audit.events": { x: 3, y: 4 } } },
      },
      schemaMapPrefs: {
        "conn-1": {
          public: {
            routing: "step",
            attrMode: "all",
            showTypes: true,
            showNulls: false,
            showComments: false,
          },
        },
      },
    });

    await useAppStore.getState().disconnectConnection("conn-1");

    expect(useAppStore.getState().connectionOverviewTab).toEqual({
      "conn-2": "details",
    });
    expect(useAppStore.getState().connectionSchemaMapSchema).toEqual({
      "conn-2": "audit",
    });
    expect(useAppStore.getState().schemaMapPositions["conn-1"]).toBeUndefined();
    expect(useAppStore.getState().schemaMapPrefs["conn-1"]).toBeUndefined();
  });

  it("deleteConnection (non-Tauri branch) drops the connection's sub-tab entry", async () => {
    mockedIsTauri.mockReturnValue(false);
    useAppStore.setState({
      connections: [connectedPostgres("conn-1"), connectedPostgres("conn-2")],
      activeConnectionId: "conn-1",
      connectionOverviewTab: {
        "conn-1": "tables",
        "conn-2": "details",
      },
      connectionSchemaMapSchema: {
        "conn-1": "public",
        "conn-2": "audit",
      },
    });

    await useAppStore.getState().deleteConnection("conn-1");

    expect(useAppStore.getState().connectionOverviewTab).toEqual({
      "conn-2": "details",
    });
    expect(useAppStore.getState().connectionSchemaMapSchema).toEqual({
      "conn-2": "audit",
    });
  });
});

describe("closeTab table browse cleanup", () => {
  it("keeps an object tab open while its connection is applying DDL", async () => {
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-object",
          kind: "object",
          label: "public.orders_view",
          connectionId: "conn-1",
          schema: "public",
          objectRef: {
            kind: "view",
            schema: "public",
            name: "orders_view",
            identityArgs: null,
          },
        },
      ],
      activeTabId: "tab-object",
      pgObjectDdlApplying: { "conn-1": true },
    });

    await useAppStore.getState().closeTab("tab-object");

    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
    expect(mockedCloseTableBrowseForTab).not.toHaveBeenCalled();
  });

  it("keeps a table designer open while its reviewed DDL is applying", async () => {
    const designerTab = {
      id: "tab-designer",
      kind: "table-designer" as const,
      label: "New table · public",
      connectionId: "conn-1",
      schema: "public",
      tableDesignerDraft: newTableDesignerForm("public"),
    };
    useAppStore.setState({
      workspaceTabs: [designerTab],
      activeTabId: designerTab.id,
    });
    useAppStore.getState().setTableDesignerApplying(designerTab.id, true);

    await useAppStore.getState().closeTab(designerTab.id);

    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
    expect(useAppStore.getState().workspaceTabs[0]?.tableDesignerApplying).toBe(
      true,
    );
    expect(mockedCloseTableBrowseForTab).not.toHaveBeenCalled();

    useAppStore.getState().setTableDesignerApplying(designerTab.id, false);
    await useAppStore.getState().closeTab(designerTab.id);
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
  });

  it("keeps an applying mutation and its tab open", async () => {
    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(true);
    const scope = tableMutationDraftScope("tab-1");
    const analysis: AnalyzeResultSetResult = {
      requestId: 1,
      analysisId: 1,
      columns: [],
      tables: [
        {
          schema: "public",
          table: "users",
          identity: { kind: "primaryKey", columns: ["id"] },
          identityProjected: true,
          identityProjectionIndexes: [],
          updatable: { allowed: true },
          deletable: { allowed: true },
          insertable: { allowed: true },
        },
      ],
      statement: { kind: "analyzed" },
    };
    const preview: PreviewResult = {
      statements: [
        {
          opIndex: 0,
          sql: "INSERT INTO public.users (name) VALUES ($1)",
          params: [{ kind: "text", value: "Ada" }],
        },
      ],
    };
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "table",
          label: "users",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
        },
      ],
      activeTabId: "tab-1",
    });
    const handle = useAppStore.getState().openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: "conn-1",
      source: { kind: "relation", schema: "public", table: "users" },
    });
    if (!handle) throw new Error("Expected mutation draft handle");
    expect(
      useAppStore.getState().setMutationDraftAnalysis(handle, analysis),
    ).toBe(true);
    useAppStore.getState().stageMutationDraftInsert(scope, {
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: "Ada" }],
    });
    const previewRequest = useAppStore
      .getState()
      .beginMutationDraftPreview(scope);
    if (!previewRequest) throw new Error("Expected mutation preview request");
    expect(
      useAppStore
        .getState()
        .resolveMutationDraftPreview(previewRequest, preview),
    ).toBe(true);
    expect(
      useAppStore.getState().beginMutationDraftApply(scope),
    ).not.toBeNull();

    await useAppStore.getState().closeTab("tab-1");

    expect(mockedRequestConfirm).not.toHaveBeenCalled();
    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
    expect(useAppStore.getState().mutationDrafts[scope]?.apply.state).toBe(
      "applying",
    );
    expect(mockedCloseTableBrowseForTab).not.toHaveBeenCalled();
  });

  it("keeps a tab and its staged changes when closing is not confirmed", async () => {
    mockedRequestConfirm.mockReset();
    mockedRequestConfirm.mockResolvedValue(false);
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "table",
          label: "users",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
        },
      ],
      activeTabId: "tab-1",
    });
    useAppStore.getState().openMutationDraft({
      owner: { kind: "table", tabId: "tab-1" },
      connectionId: "conn-1",
      source: { kind: "relation", schema: "public", table: "users" },
    });
    useAppStore.getState().stageMutationDraftInsert("table:tab-1", {
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: "Ada" }],
    });

    await useAppStore.getState().closeTab("tab-1");

    expect(mockedRequestConfirm).toHaveBeenCalledOnce();
    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);
    expect(useAppStore.getState().mutationDrafts["table:tab-1"]).toBeDefined();
    expect(mockedCloseTableBrowseForTab).not.toHaveBeenCalled();
  });

  it("awaits closeTableBrowseForTab before dropping the tab", async () => {
    let resolveClose: (() => void) | undefined;
    mockedCloseTableBrowseForTab.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveClose = resolve;
        }),
    );
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "table",
          label: "users",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
        },
      ],
      activeTabId: "tab-1",
      tableBrowses: {
        "tab-1": {
          tabId: "tab-1",
          connectionId: "conn-1",
          schema: "public",
          table: "users",
          generation: 1,
          typedFilters: [],
          rawFilterText: "",
          filterMode: "typed",
          sort: [],
          pageSize: 100,
          page: 1,
          cursorStack: [],
          nextRequestToken: 0,
          inflightRequestId: null,
          appliedRequestId: null,
          result: null,
          loadStatus: { state: "idle" },
          countStatus: { state: "idle" },
          exactCount: null,
          prefsLoaded: true,
          prefs: defaultTableGridPrefs(),
        },
      },
    });

    const closePromise = useAppStore.getState().closeTab("tab-1");
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (mockedCloseTableBrowseForTab.mock.calls.length > 0) break;
      await Promise.resolve();
    }
    expect(mockedCloseTableBrowseForTab).toHaveBeenCalledWith(
      "conn-1",
      "tab-1",
    );
    expect(useAppStore.getState().workspaceTabs).toHaveLength(1);

    resolveClose?.();
    await closePromise;

    expect(useAppStore.getState().workspaceTabs).toEqual([]);
    expect(useAppStore.getState().tableBrowses["tab-1"]).toBeUndefined();
  });
});
