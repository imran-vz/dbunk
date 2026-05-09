import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => {
  return {
    isTauri: vi.fn(() => true),
    tauriInvoke: vi.fn(),
  };
});

import { useAppStore } from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedIsTauri = vi.mocked(isTauri);

const tableDataKey = (connectionId: string, schema: string, table: string) =>
  `${connectionId}::${schema}::${table}`;

const resetStore = () => {
  useAppStore.setState({
    activeConnectionId: "conn-1",
    tableData: {},
    tableLoadStatus: {},
    workspaceTabs: [],
  });
};

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  resetStore();
});

afterEach(() => {
  mockedInvoke.mockReset();
});

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
    expect(state.tableLoadStatus[key]).toEqual({ status: "success" });
  });

  it("records an error when the invoke rejects", async () => {
    mockedInvoke.mockRejectedValueOnce(new Error("connection lost"));

    await useAppStore.getState().loadTableData("conn-1", "public", "users");

    const key = tableDataKey("conn-1", "public", "users");
    const status = useAppStore.getState().tableLoadStatus[key];
    expect(status?.status).toBe("error");
    if (status?.status !== "error") {
      throw new Error("expected error status");
    }
    expect(status.message).toContain("connection lost");
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
    expect(useAppStore.getState().tableLoadStatus[key]?.status).toBe("loading");

    resolveInvoke?.({
      columns: [],
      rows: [],
      page: 1,
      pageSize: 100,
      totalRows: 0,
      runtimeMs: 1,
    });
    await promise;

    expect(useAppStore.getState().tableLoadStatus[key]?.status).toBe("success");
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
