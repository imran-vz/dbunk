// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  resetSessionPersistenceForTests,
  SESSION_STORAGE_KEY,
  startSessionPersistence,
} from "@/lib/session-persistence";
import { useAppStore, type Connection, type WorkspaceTab } from "@/lib/store";

// Non-Tauri: the ui-state store passes through to localStorage, so the
// session blob is directly observable there.

const initialStoreState = useAppStore.getState();

const connection = (id: string): Connection => ({
  id,
  name: id,
  database: "app",
  host: "h",
  port: 5432,
  user: "u",
  password: "",
  role: "",
  engine: "PostgreSQL",
  ssl: false,
  status: "Connected",
  latency: "1ms",
});

const queryTab = (id: string, query: string): WorkspaceTab => ({
  id,
  kind: "query",
  label: `${id}.sql`,
  connectionId: "conn-1",
  schema: "public",
  query,
  isDirty: true,
});

beforeEach(() => {
  window.localStorage.clear();
  resetSessionPersistenceForTests();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
  vi.useRealTimers();
});

describe("restoreSession (P8)", () => {
  it("rebuilds tabs, active tab, and expanded nodes from the blob", () => {
    window.localStorage.setItem(
      SESSION_STORAGE_KEY,
      JSON.stringify({
        tabs: [
          {
            id: "tab-4",
            kind: "query",
            label: "query_7.sql",
            connectionId: "conn-1",
            schema: "public",
            query: "select 1;",
            isDirty: true,
          },
          {
            id: "tab-5",
            kind: "table",
            label: "users",
            connectionId: "conn-1",
            schema: "public",
            table: "users",
            pinned: true,
          },
          // Dropped: its connection no longer exists.
          {
            id: "tab-6",
            kind: "query",
            label: "gone.sql",
            connectionId: "conn-gone",
            schema: "public",
          },
          // Dropped: malformed entry.
          { id: 42 },
        ],
        activeTabId: "tab-5",
        expandedSchemas: ["conn-1:public"],
      }),
    );
    useAppStore.setState({ connections: [connection("conn-1")] });

    useAppStore.getState().restoreSession();

    const state = useAppStore.getState();
    expect(state.workspaceTabs.map((tab) => tab.id)).toEqual([
      "tab-4",
      "tab-5",
    ]);
    // Hot-exit SQL restored exactly.
    expect(state.workspaceTabs[0]?.query).toBe("select 1;");
    expect(state.workspaceTabs[0]?.isDirty).toBe(true);
    expect(state.workspaceTabs[1]?.pinned).toBe(true);
    expect(state.activeTabId).toBe("tab-5");
    expect(state.expandedSchemas).toEqual(["conn-1:public"]);

    // Counters bumped past restored ids/labels — no collisions.
    useAppStore.getState().createNewQueryTab();
    const created = useAppStore.getState().workspaceTabs.at(-1);
    expect(created?.id).not.toBe("tab-4");
    expect(created?.id).not.toBe("tab-5");
    expect(created?.label).not.toBe("query_7.sql");
  });

  it("restores a valid caret and degrades malformed carets to none (Plan 009)", () => {
    window.localStorage.setItem(
      SESSION_STORAGE_KEY,
      JSON.stringify({
        tabs: [
          {
            id: "tab-1",
            kind: "query",
            label: "query_1.sql",
            connectionId: "conn-1",
            schema: "public",
            query: "select 1;",
            caret: { line: 3, column: 7, anchorLine: 1, anchorColumn: 2 },
          },
          {
            id: "tab-2",
            kind: "query",
            label: "query_2.sql",
            connectionId: "conn-1",
            schema: "public",
            // Malformed: zero/negative/fractional members. Tab must
            // survive with no caret — the field degrades, not the tab.
            caret: { line: 0, column: -1 },
          },
          {
            id: "tab-3",
            kind: "query",
            label: "query_3.sql",
            connectionId: "conn-1",
            schema: "public",
            // Half-valid anchor is dropped; primary position kept.
            caret: { line: 2, column: 4, anchorLine: 1.5 },
          },
        ],
        activeTabId: "tab-1",
        expandedSchemas: [],
      }),
    );
    useAppStore.setState({ connections: [connection("conn-1")] });

    useAppStore.getState().restoreSession();

    const [first, second, third] = useAppStore.getState().workspaceTabs;
    expect(first?.caret).toEqual({
      line: 3,
      column: 7,
      anchorLine: 1,
      anchorColumn: 2,
    });
    expect(second?.id).toBe("tab-2");
    expect(second?.caret).toBeUndefined();
    expect(third?.caret).toEqual({ line: 2, column: 4 });
  });

  it("falls back silently on corrupt data", () => {
    window.localStorage.setItem(SESSION_STORAGE_KEY, "{not json!");
    useAppStore.setState({ connections: [connection("conn-1")] });
    useAppStore.getState().restoreSession();
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
  });

  it("is a no-op without a stored session", () => {
    useAppStore.getState().restoreSession();
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
  });

  it("does not emit a tab-reveal signal (the persisted rail wins at boot)", () => {
    window.localStorage.setItem(
      SESSION_STORAGE_KEY,
      JSON.stringify({
        tabs: [queryTab("tab-1", "select 1;")],
        activeTabId: "tab-1",
        expandedSchemas: [],
      }),
    );
    useAppStore.setState({ connections: [connection("conn-1")] });
    const before = useAppStore.getState().tabRevealRequest;

    useAppStore.getState().restoreSession();
    expect(useAppStore.getState().tabRevealRequest).toBe(before);

    // Explicit opens/activations do emit, so the workbench can leave
    // rails that don't render tabs.
    useAppStore.getState().setActiveTabId("tab-1");
    expect(useAppStore.getState().tabRevealRequest).toBe(before + 1);
    useAppStore.getState().createNewQueryTab();
    expect(useAppStore.getState().tabRevealRequest).toBe(before + 2);
  });
});

describe("session persistence (P8)", () => {
  it("writes the session blob (debounced) when tabs change", () => {
    vi.useFakeTimers();
    startSessionPersistence();

    useAppStore.setState({
      workspaceTabs: [queryTab("tab-1", "select 42;")],
      activeTabId: "tab-1",
    });

    expect(window.localStorage.getItem(SESSION_STORAGE_KEY)).toBeNull();
    vi.advanceTimersByTime(600);

    const raw = window.localStorage.getItem(SESSION_STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw ?? "{}");
    expect(parsed.activeTabId).toBe("tab-1");
    expect(parsed.tabs).toEqual([
      {
        id: "tab-1",
        kind: "query",
        label: "tab-1.sql",
        connectionId: "conn-1",
        schema: "public",
        query: "select 42;",
        isDirty: true,
      },
    ]);
  });

  it("persists the caret recorded via updateQueryCaret (Plan 009)", () => {
    vi.useFakeTimers();
    startSessionPersistence();

    // Deliberately not dirty: the assertions below must prove that
    // moving the caret alone never flips the dirty flag.
    const cleanTab = { ...queryTab("tab-1", "select 42;") };
    delete cleanTab.isDirty;
    useAppStore.setState({
      connections: [connection("conn-1")],
      workspaceTabs: [cleanTab],
      activeTabId: "tab-1",
    });
    useAppStore.getState().updateQueryCaret("tab-1", { line: 2, column: 9 });
    // No-ops: unknown tab and non-query tabs are ignored.
    useAppStore
      .getState()
      .updateQueryCaret("tab-ghost", { line: 1, column: 1 });
    vi.advanceTimersByTime(600);

    const parsed = JSON.parse(
      window.localStorage.getItem(SESSION_STORAGE_KEY) ?? "{}",
    );
    expect(parsed.tabs[0].caret).toEqual({ line: 2, column: 9 });
    expect(parsed.tabs[0].isDirty).toBeUndefined();
    // Moving the caret alone must not mark the tab dirty.
    const tab = useAppStore.getState().workspaceTabs[0];
    expect(tab.caret).toEqual({ line: 2, column: 9 });
    expect(tab.isDirty).toBeUndefined();
  });

  it("excludes keyvalue-workspace tabs from the blob", () => {
    vi.useFakeTimers();
    startSessionPersistence();
    useAppStore.setState({
      workspaceTabs: [
        queryTab("tab-1", "select 1;"),
        {
          id: "tab-2",
          kind: "key",
          label: "user:1",
          connectionId: "conn-2",
          schema: "",
        },
      ],
      activeTabId: "tab-1",
    });
    vi.advanceTimersByTime(600);
    const parsed = JSON.parse(
      window.localStorage.getItem(SESSION_STORAGE_KEY) ?? "{}",
    );
    expect(parsed.tabs).toHaveLength(1);
    expect(parsed.tabs[0].id).toBe("tab-1");
  });

  it("sheds the largest hot-exit SQL when the session exceeds the value budget", () => {
    vi.useFakeTimers();
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    startSessionPersistence();

    // One pasted SQL dump larger than the whole per-value limit, plus a
    // normal tab whose SQL must survive.
    useAppStore.setState({
      workspaceTabs: [
        {
          ...queryTab("tab-1", "x".repeat(600 * 1024)),
          caret: { line: 4000, column: 1 },
        },
        queryTab("tab-2", "select 2;"),
      ],
      activeTabId: "tab-2",
    });
    vi.advanceTimersByTime(600);

    const raw = window.localStorage.getItem(SESSION_STORAGE_KEY);
    expect(raw).not.toBeNull();
    const parsed = JSON.parse(raw ?? "{}");
    expect(parsed.tabs).toHaveLength(2);
    expect(parsed.tabs[0].id).toBe("tab-1");
    expect(parsed.tabs[0].query).toBeUndefined();
    // Shedding the SQL must also shed the caret and dirty flag — an
    // orphaned caret and a false "unsaved changes" claim otherwise
    // restore against an empty editor.
    expect(parsed.tabs[0].caret).toBeUndefined();
    expect(parsed.tabs[0].isDirty).toBeUndefined();
    expect(parsed.tabs[1].query).toBe("select 2;");
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });
});
