import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import {
  type Connection,
  tableDataKey,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedIsTauri = vi.mocked(isTauri);

const initialStoreState = useAppStore.getState();

const resetStore = () => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ activeConnectionId: "conn-1" });
};

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  resetStore();
});

afterEach(() => {
  resetStore();
});

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
    expect(state.tableLoadStatus.users).toEqual({ state: "success" });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("connection lost"));

    await useAppStore.getState().loadTableData("conn-1", "public", "users");

    const status = useAppStore.getState().tableLoadStatus.users;
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

    expect(useAppStore.getState().tableLoadStatus.users?.state).toBe("loading");

    resolveInvoke?.({
      columns: [],
      rows: [],
      page: 1,
      pageSize: 100,
      totalRows: 0,
      runtimeMs: 1,
    });
    await promise;

    expect(useAppStore.getState().tableLoadStatus.users?.state).toBe("success");
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
});

describe("runQuery status tracking", () => {
  it("transitions from running to success and stores runtime", async () => {
    const tabId = seedQueryTab();

    let resolveInvoke: ((value: unknown) => void) | null = null;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveInvoke = resolve;
        }),
    );

    const runPromise = act(async () => {
      void useAppStore.getState().runQuery(tabId);
    });
    await runPromise;

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

    const status = useAppStore.getState().queryStatus[tabId];
    if (status.state !== "success") {
      throw new Error(`expected success status, got ${status.state}`);
    }
    expect(status.runtimeMs).toBe(42);
  });

  it("transitions to error and captures error message on failure", async () => {
    const tabId = seedQueryTab();

    mockedInvoke.mockRejectedValueOnce(new Error("syntax error at or near"));

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const status = useAppStore.getState().queryStatus[tabId];
    if (status.state !== "error") {
      throw new Error(`expected error status, got ${status.state}`);
    }
    expect(status.error).toContain("syntax error");
  });

  it("captures string error rejection messages", async () => {
    const tabId = seedQueryTab();
    mockedInvoke.mockRejectedValueOnce("connection lost");

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    expect(useAppStore.getState().queryStatus[tabId]).toEqual({
      state: "error",
      error: "connection lost",
    });
  });

  it("ignores re-entry while a tab query is running", async () => {
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

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });
    expect(mockedInvoke).toHaveBeenCalledTimes(1);

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

    expect(useAppStore.getState().tableLoadStatus.users).toEqual({
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

    expect(useAppStore.getState().tableLoadStatus.users.state).toBe("success");
  });

  it("captures error message on failure", async () => {
    useAppStore.setState({ activeConnectionId: "conn-1" });

    mockedInvoke.mockRejectedValueOnce(new Error("relation does not exist"));

    await act(async () => {
      await useAppStore.getState().loadTablePreview("public", "missing");
    });

    const status = useAppStore.getState().tableLoadStatus.missing;
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
    expect(state.tableStructure[key]?.capabilities).toEqual({
      columns: true,
      primaryKey: true,
      foreignKeys: false,
      indexes: false,
      constraints: false,
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
          lastSync: "Never",
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
          lastSync: "Never",
          errorMessage: "previous failure",
        },
      ],
    });

    mockedInvoke
      .mockResolvedValueOnce({ latencyMs: 10 })
      .mockResolvedValueOnce([]);

    await act(async () => {
      await useAppStore.getState().connectConnection("conn-1");
    });

    const connection = useAppStore
      .getState()
      .connections.find((c) => c.id === "conn-1");
    expect(connection?.status).toBe("Connected");
    expect(connection?.errorMessage).toBeUndefined();
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
          lastSync: "Never",
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

  it("stores result in queryPreviews keyed by tab.label", async () => {
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

    const preview = useAppStore.getState().queryPreviews["query_42.sql"];
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
          lastSync: "Never",
        },
      ],
    });
  };

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

    await act(async () => {
      await useAppStore.getState().runQuery(tabId);
    });

    const status = useAppStore.getState().queryStatus[tabId];
    if (status.state !== "success") {
      throw new Error(`expected success, got ${status.state}`);
    }
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
    lastSync: "Just now",
  };
  useAppStore.setState({
    connections: [connection],
    activeConnectionId: connection.id,
  });
  return connection;
};

describe("store.pendingStructureChanges", () => {
  const key = tableStructureKey("conn-1", "public", "users");

  it("addPendingStructureChange appends a change and assigns it an id", () => {
    const change: ColumnChangeKind = { kind: "drop", columnName: "legacy" };
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change,
      });
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending).toHaveLength(1);
    expect(pending[0].change).toEqual(change);
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
        change: a,
      });
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: b,
      });
    });
    const pending = useAppStore.getState().pendingStructureChanges[key] ?? [];
    expect(pending.map((p) => p.change)).toEqual([a, b]);
  });

  it("removePendingStructureChange removes only the targeted entry", () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: { kind: "drop", columnName: "a" },
      });
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: { kind: "drop", columnName: "b" },
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
        change: { kind: "drop", columnName: "a" },
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
    await useAppStore.getState().commitStructureChanges(key);
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("invokes execute_ddl with generated SQL and clears pending on success", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: { kind: "drop", columnName: "legacy" },
      });
    });
    // First call: execute_ddl, second call: load_table_structure refresh.
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
        },
      });

    await useAppStore.getState().commitStructureChanges(key);

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
    expect(state.structureCommitStatus[key]?.state).toBe("success");
  });

  it("preserves pending and surfaces error on backend failure", async () => {
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: { kind: "drop", columnName: "legacy" },
      });
    });
    mockedInvoke.mockRejectedValueOnce(new Error("permission denied"));

    await useAppStore.getState().commitStructureChanges(key);

    const state = useAppStore.getState();
    expect(state.pendingStructureChanges[key] ?? []).toHaveLength(1);
    expect(state.structureCommitStatus[key]).toEqual({
      state: "error",
      error: "permission denied",
    });
    // Should not refresh structure on failure.
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
  });

  it("rejects commit when active engine is not PostgreSQL", async () => {
    useAppStore.setState((state) => ({
      connections: state.connections.map((c) =>
        c.id === "conn-1" ? { ...c, engine: "MySQL" } : c,
      ),
    }));
    act(() => {
      useAppStore.getState().addPendingStructureChange(key, {
        schema: "public",
        table: "users",
        change: { kind: "drop", columnName: "legacy" },
      });
    });

    await useAppStore.getState().commitStructureChanges(key);

    expect(mockedInvoke).not.toHaveBeenCalled();
    const status = useAppStore.getState().structureCommitStatus[key];
    expect(status?.state).toBe("error");
    if (status?.state === "error") {
      expect(status.error).toMatch(/postgres/i);
    }
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
          capabilities: {
            columns: true,
            primaryKey: true,
            foreignKeys: true,
            indexes: true,
            constraints: true,
          },
        },
      },
    });
  };

  it("does nothing when there are no pending edits", async () => {
    seedTable();
    await useAppStore.getState().commitTableEdits("users");
    expect(mockedInvoke).not.toHaveBeenCalled();
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

    await useAppStore.getState().commitTableEdits("users");

    const state = useAppStore.getState();
    expect(state.tableEdits["users"]).toBeUndefined();
    expect(state.tableEditsCommitStatus["users"]?.state).toBe("success");
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

  it("preserves edits and reports an error on commit failure", async () => {
    seedTable();
    act(() => {
      useAppStore.getState().setTableEdit("users", 0, 1, "ada@new.com");
    });
    mockedInvoke.mockRejectedValueOnce(new Error("row not found: id=1"));

    await useAppStore.getState().commitTableEdits("users");

    const state = useAppStore.getState();
    expect(state.tableEdits["users"]?.[0]?.[1]).toBe("ada@new.com");
    const status = state.tableEditsCommitStatus["users"];
    if (status?.state !== "error") {
      throw new Error(`expected error status, got ${status?.state}`);
    }
    expect(status.error).toContain("row not found");
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

    await useAppStore.getState().commitTableEdits("users");

    expect(mockedInvoke).not.toHaveBeenCalled();
    const status = useAppStore.getState().tableEditsCommitStatus["users"];
    if (status?.state !== "error") {
      throw new Error(`expected error status, got ${status?.state}`);
    }
    expect(status.error).toMatch(/read.?only|primary key|unique/i);
    // Edits are kept so the user can still discard explicitly.
    expect(useAppStore.getState().tableEdits["users"]?.[0]?.[1]).toBe(
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
    const payload = (call?.[1] as { payload: { edits: unknown[] } }).payload;
    expect(payload.edits).toHaveLength(1);
    expect(payload.edits[0]).toMatchObject({
      rowIndex: 1,
      identity: [{ column: "id", value: "2" }],
      set: [{ column: "name", value: "Grace H." }],
    });
  });
});
