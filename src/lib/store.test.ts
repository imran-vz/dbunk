import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
}));

import { isTauri, tauriInvoke } from "@/lib/tauri";
import { useAppStore, type WorkspaceTab } from "@/lib/store";

const mockedInvoke = vi.mocked(tauriInvoke);
const mockedIsTauri = vi.mocked(isTauri);

const TAB_ID = "tab-test-1";

const buildQueryTab = (overrides?: Partial<WorkspaceTab>): WorkspaceTab => ({
  id: TAB_ID,
  kind: "query",
  label: "query_test.sql",
  connectionId: "conn-1",
  schema: "public",
  query: "select * from users;",
  ...overrides,
});

const seedTab = (tab: WorkspaceTab) => {
  useAppStore.setState({ workspaceTabs: [tab] });
};

const getTab = (id: string) =>
  useAppStore.getState().workspaceTabs.find((t) => t.id === id);

beforeEach(() => {
  // Reset zustand state to a known shape
  useAppStore.setState({
    workspaceTabs: [],
    queryPreviews: {},
    recentQueries: [],
  });
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("runQuery", () => {
  it("runs the tab.query when no overrideSql is supplied", async () => {
    seedTab(buildQueryTab({ query: "select * from users;" }));
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id"],
      rows: [["1"]],
      runtimeMs: 12,
      rowCount: 1,
    });

    await useAppStore.getState().runQuery(TAB_ID);

    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
      payload: { connectionId: "conn-1", query: "select * from users;" },
    });
  });

  it("runs the overrideSql when one is supplied", async () => {
    seedTab(buildQueryTab({ query: "select * from users;" }));
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      runtimeMs: 1,
      rowCount: 0,
    });

    await useAppStore
      .getState()
      .runQuery(TAB_ID, { overrideSql: "SELECT 1" });

    expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
      payload: { connectionId: "conn-1", query: "SELECT 1" },
    });
  });

  it("falls back to tab.query when overrideSql is empty or whitespace", async () => {
    seedTab(buildQueryTab({ query: "select * from users;" }));
    mockedInvoke.mockResolvedValue({
      columns: [],
      rows: [],
      runtimeMs: 0,
      rowCount: 0,
    });

    await useAppStore.getState().runQuery(TAB_ID, { overrideSql: "" });
    await useAppStore.getState().runQuery(TAB_ID, { overrideSql: "   \n" });

    for (const call of mockedInvoke.mock.calls) {
      expect(call[0]).toBe("run_query");
      expect(call[1]).toEqual({
        payload: { connectionId: "conn-1", query: "select * from users;" },
      });
    }
    expect(mockedInvoke).toHaveBeenCalledTimes(2);
  });

  it("does not mutate tab.query when overrideSql is used", async () => {
    seedTab(buildQueryTab({ query: "select * from users;" }));
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      runtimeMs: 5,
      rowCount: 0,
    });

    await useAppStore
      .getState()
      .runQuery(TAB_ID, { overrideSql: "SELECT 1" });

    expect(getTab(TAB_ID)?.query).toBe("select * from users;");
  });

  it("records the executed SQL (override) in recentQueries", async () => {
    seedTab(buildQueryTab({ query: "select * from users;" }));
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      runtimeMs: 5,
      rowCount: 0,
    });

    await useAppStore
      .getState()
      .runQuery(TAB_ID, { overrideSql: "SELECT 1" });

    expect(useAppStore.getState().recentQueries[0]).toBe("SELECT 1");
  });

  it("updates lastRun and clears isDirty on success", async () => {
    seedTab(buildQueryTab({ isDirty: true }));
    mockedInvoke.mockResolvedValueOnce({
      columns: [],
      rows: [],
      runtimeMs: 5,
      rowCount: 0,
    });

    await useAppStore.getState().runQuery(TAB_ID);

    const tab = getTab(TAB_ID);
    expect(tab?.lastRun).toBe("Just now");
    expect(tab?.isDirty).toBe(false);
  });

  it("stores result in queryPreviews keyed by tab.label", async () => {
    seedTab(buildQueryTab({ label: "query_42.sql" }));
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id", "name"],
      rows: [["1", "Ada"]],
      runtimeMs: 17,
      rowCount: 1,
    });

    await useAppStore
      .getState()
      .runQuery(TAB_ID, { overrideSql: "SELECT 1" });

    const preview = useAppStore.getState().queryPreviews["query_42.sql"];
    expect(preview).toEqual({
      columns: ["id", "name"],
      rows: [["1", "Ada"]],
      runtime: "17 ms",
      rowCount: "1",
      cache: "Cold",
    });
  });

  it("marks lastRun as Failed when invoke rejects", async () => {
    seedTab(buildQueryTab());
    mockedInvoke.mockRejectedValueOnce(new Error("boom"));
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

    await useAppStore.getState().runQuery(TAB_ID);

    expect(getTab(TAB_ID)?.lastRun).toBe("Failed");
    consoleSpy.mockRestore();
  });

  it("ignores the call when tab is not a query tab", async () => {
    seedTab(buildQueryTab({ kind: "table", query: undefined }));

    await useAppStore
      .getState()
      .runQuery(TAB_ID, { overrideSql: "SELECT 1" });

    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("ignores the call when neither override nor tab.query has SQL", async () => {
    seedTab(buildQueryTab({ query: "" }));

    await useAppStore.getState().runQuery(TAB_ID, { overrideSql: "  " });

    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});
