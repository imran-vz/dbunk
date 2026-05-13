/**
 * Relational Queries slice — owns Query History, Saved Queries, and
 * the per-tab query editor / run state.
 *
 * Exposes `dropQueryStateForConnection(connectionId)` as its piece of
 * the delete-connection cleanup cascade — drops query-history rows
 * pinned to that connection (we don't currently key queryStatus or
 * queryPreviews by connectionId, so they aren't cleaned here).
 */

import type { StateCreator } from "zustand";

import { pickSqlToRun } from "@/lib/sql";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  QueryHistoryEntry,
  QueryOutcome,
  QueryPreviewData,
  QueryStatus,
  SavedQueriesStatus,
  SavedQuery,
} from "./types";

type RunQueryResult = {
  columns: string[];
  rows: string[][];
  runtimeMs: number;
  rowCount: number;
};

const generateHistoryId = (): string => {
  if (
    typeof globalThis !== "undefined" &&
    typeof (globalThis as { crypto?: { randomUUID?: () => string } }).crypto
      ?.randomUUID === "function"
  ) {
    return (
      globalThis as { crypto: { randomUUID: () => string } }
    ).crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
};

export type RelationalQueriesSlice = {
  queryEdits: Record<string, Record<number, Record<number, string>>>;
  queryStatus: Record<string, QueryStatus>;
  queryPreviews: Record<string, QueryPreviewData>;
  queryHistory: QueryHistoryEntry[];
  savedQueries: SavedQuery[];
  savedQueriesStatus: SavedQueriesStatus;

  setQueryEdit: (
    tabId: string,
    rowIndex: number,
    colIndex: number,
    value: string,
  ) => void;
  discardQueryEdits: (tabId: string) => void;
  updateQuery: (tabId: string, query: string) => void;
  runQuery: (
    tabId: string,
    options?: { overrideSql?: string },
  ) => Promise<QueryOutcome>;
  loadQueryHistory: () => Promise<void>;
  loadSavedQueries: () => Promise<void>;
  saveSavedQuery: (
    query: Omit<SavedQuery, "createdAt" | "updatedAt"> &
      Partial<Pick<SavedQuery, "createdAt" | "updatedAt">>,
  ) => Promise<void>;
  deleteSavedQuery: (id: string) => Promise<void>;

  /**
   * Cascade cleanup for closing query tabs without removing the
   * persistent query history list.
   */
  dropOpenQueryStateForConnection: (connectionId: string) => void;

  /**
   * Cascade cleanup — drops query-history rows for that connection
   * and any query-status entries for its open tabs (the tab cleanup
   * is best-effort because queryStatus is keyed by tab ID, not
   * connection ID, so we look up the open tabs and drop their
   * status entries).
   */
  dropQueryStateForConnection: (connectionId: string) => void;
};

export const createRelationalQueriesSlice: StateCreator<
  AppStoreState,
  [],
  [],
  RelationalQueriesSlice
> = (set, get) => ({
  queryEdits: {},
  queryStatus: {},
  queryPreviews: {},
  queryHistory: [],
  savedQueries: [],
  savedQueriesStatus: { state: "idle" },

  setQueryEdit: (tabId, rowIndex, colIndex, value) =>
    set((state) => ({
      queryEdits: {
        ...state.queryEdits,
        [tabId]: {
          ...state.queryEdits[tabId],
          [rowIndex]: {
            ...state.queryEdits[tabId]?.[rowIndex],
            [colIndex]: value,
          },
        },
      },
    })),

  discardQueryEdits: (tabId) =>
    set((state) => {
      const { [tabId]: _, ...rest } = state.queryEdits;
      return { queryEdits: rest };
    }),

  updateQuery: (tabId, query) =>
    set((state) => ({
      workspaceTabs: state.workspaceTabs.map((tab) =>
        tab.id === tabId ? { ...tab, query, isDirty: true } : tab,
      ),
    })),

  // cyclo/cog under threshold; CRAP stays high because fallow's
  // static_estimated model caps slice methods at the file's "partial"
  // tier regardless of branch tests (6 in store.test.ts cover all paths).
  // fallow-ignore-next-line complexity
  runQuery: async (tabId, options): Promise<QueryOutcome> => {
    const state = get();
    const tab = state.workspaceTabs.find((item) => item.id === tabId);
    if (!tab || tab.kind !== "query") {
      return { kind: "noop" };
    }
    if (state.queryStatus[tabId]?.state === "running") {
      return { kind: "noop" };
    }
    const fullText = tab.query ?? "";
    const overrideSql = options?.overrideSql ?? null;
    const query = pickSqlToRun(fullText, overrideSql).trim();
    if (!query) {
      return { kind: "noop" };
    }
    if (!isTauri()) {
      return { kind: "noop" };
    }
    const connectionAtRun = state.connections.find(
      (c) => c.id === tab.connectionId,
    );
    const startedAt = new Date().toISOString();
    set((s) => ({
      queryStatus: {
        ...s.queryStatus,
        [tabId]: { state: "running" },
      },
    }));
    const buildEntry = (
      base: Pick<QueryHistoryEntry, "status" | "runtimeMs"> &
        Partial<Pick<QueryHistoryEntry, "rowCount" | "errorMessage">>,
    ): QueryHistoryEntry => ({
      id: generateHistoryId(),
      sql: query,
      connectionId: tab.connectionId,
      connectionName: connectionAtRun?.name ?? "",
      database: connectionAtRun?.database ?? "",
      engine: connectionAtRun?.engine ?? "PostgreSQL",
      status: base.status,
      errorMessage: base.errorMessage,
      runtimeMs: base.runtimeMs,
      rowCount: base.rowCount,
      startedAt,
    });
    const persistEntry = async (entry: QueryHistoryEntry) => {
      try {
        await tauriInvoke<QueryHistoryEntry[]>("append_query_history", {
          entry,
        });
      } catch (error) {
        console.error("Failed to persist query history entry", error);
      }
    };
    try {
      const result = await tauriInvoke<RunQueryResult>("run_query", {
        payload: { connectionId: tab.connectionId, query },
      });
      const entry = buildEntry({
        status: "success",
        runtimeMs: result.runtimeMs,
        rowCount: result.rowCount,
      });
      const nowIso = new Date().toISOString();
      set((s) => {
        const { [tabId]: _status, ...restStatus } = s.queryStatus;
        return {
          queryPreviews: {
            ...s.queryPreviews,
            [tab.label]: {
              columns: result.columns,
              rows: result.rows,
              runtime: `${result.runtimeMs} ms`,
              rowCount: result.rowCount.toString(),
              cache: "Cold",
            },
          },
          queryStatus: restStatus,
          queryHistory: [entry, ...s.queryHistory].slice(0, 200),
          workspaceTabs: s.workspaceTabs.map((item) =>
            item.id === tabId
              ? { ...item, lastRun: "Just now", isDirty: false }
              : item,
          ),
          // Cross-slice write: bumps the Connection's lastActivityAt.
          // Belongs as `get().applyConnectionActivity()` once the
          // Connections slice exposes a helper for it (follow-up).
          connections: s.connections.map((c) =>
            c.id === tab.connectionId ? { ...c, lastActivityAt: nowIso } : c,
          ),
        };
      });
      await persistEntry(entry);
      return {
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowCount: result.rowCount,
      };
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to run query", error);
      const entry = buildEntry({
        status: "error",
        runtimeMs: Math.max(0, Date.now() - new Date(startedAt).getTime()),
        errorMessage: message,
      });
      // Single set: drop the queryStatus lifecycle entry in the same
      // updater that writes queryHistory + workspaceTabs, so the
      // failure path matches the success-path shape (one set per
      // terminal transition).
      set((s) => {
        const { [tabId]: _status, ...restStatus } = s.queryStatus;
        return {
          queryStatus: restStatus,
          queryHistory: [entry, ...s.queryHistory].slice(0, 200),
          workspaceTabs: s.workspaceTabs.map((item) =>
            item.id === tabId
              ? { ...item, lastRun: "Failed", isDirty: false }
              : item,
          ),
        };
      });
      await persistEntry(entry);
      return { kind: "failed", reason: message };
    }
  },

  loadQueryHistory: async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const stored =
        await tauriInvoke<QueryHistoryEntry[]>("load_query_history");
      set({ queryHistory: stored });
    } catch (error) {
      console.error("Failed to load query history", error);
    }
  },

  loadSavedQueries: async () => {
    if (!isTauri()) {
      set({ savedQueriesStatus: { state: "success" } });
      return;
    }
    set({ savedQueriesStatus: { state: "loading" } });
    try {
      const stored = await tauriInvoke<SavedQuery[]>("load_saved_queries");
      set({ savedQueries: stored, savedQueriesStatus: { state: "success" } });
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load saved queries", error);
      set({
        savedQueriesStatus: { state: "error", error: message },
      });
    }
  },

  saveSavedQuery: async (input) => {
    const now = new Date().toISOString();
    const payload: SavedQuery = {
      id: input.id,
      name: input.name,
      body: input.body,
      connectionId: input.connectionId,
      isFavorite: input.isFavorite,
      ownerId: input.ownerId ?? null,
      createdAt: input.createdAt ?? now,
      updatedAt: input.updatedAt ?? now,
    };
    if (!isTauri()) {
      set((state) => ({
        savedQueries: [
          payload,
          ...state.savedQueries.filter((q) => q.id !== payload.id),
        ],
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<SavedQuery[]>("save_saved_query", {
        query: payload,
      });
      set({ savedQueries: stored });
    } catch (error) {
      console.error("Failed to save saved query", error);
    }
  },

  deleteSavedQuery: async (id) => {
    if (!isTauri()) {
      set((state) => ({
        savedQueries: state.savedQueries.filter((q) => q.id !== id),
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<SavedQuery[]>("delete_saved_query", {
        payload: { id },
      });
      set({ savedQueries: stored });
    } catch (error) {
      console.error("Failed to delete saved query", error);
    }
  },

  dropOpenQueryStateForConnection: (connectionId) =>
    set((state) => {
      const tabsForConnection = state.workspaceTabs.filter(
        (tab) => tab.connectionId === connectionId,
      );
      const tabIdSet = new Set(tabsForConnection.map((tab) => tab.id));
      const queryLabelSet = new Set(
        tabsForConnection
          .filter((tab) => tab.kind === "query")
          .map((tab) => tab.label),
      );
      const filterByTab = <T>(bag: Record<string, T>): Record<string, T> => {
        const next: Record<string, T> = {};
        for (const [key, value] of Object.entries(bag)) {
          if (!tabIdSet.has(key)) {
            next[key] = value;
          }
        }
        return next;
      };
      const filterByQueryLabel = <T>(
        bag: Record<string, T>,
      ): Record<string, T> => {
        const next: Record<string, T> = {};
        for (const [key, value] of Object.entries(bag)) {
          if (!queryLabelSet.has(key)) {
            next[key] = value;
          }
        }
        return next;
      };
      return {
        queryStatus: filterByTab(state.queryStatus),
        queryEdits: filterByTab(state.queryEdits),
        queryPreviews: filterByQueryLabel(state.queryPreviews),
      };
    }),

  dropQueryStateForConnection: (connectionId) => {
    get().dropOpenQueryStateForConnection(connectionId);
    set((state) => ({
      queryHistory: state.queryHistory.filter(
        (entry) => entry.connectionId !== connectionId,
      ),
    }));
  },
});
