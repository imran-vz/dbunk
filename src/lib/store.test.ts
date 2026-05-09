import { act } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

import { tableDataKey, useAppStore } from "@/lib/store";
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
