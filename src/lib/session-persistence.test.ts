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
        queryTab("tab-1", "x".repeat(600 * 1024)),
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
    expect(parsed.tabs[1].query).toBe("select 2;");
    expect(warnSpy).toHaveBeenCalled();
    warnSpy.mockRestore();
  });
});
