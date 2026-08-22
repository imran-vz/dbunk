/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/require-safety-comment-for-type-assertion -- Persisted Zustand state is an external boundary migrated and validated in this module. */
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

import { querySessionChannelsAvailable } from "@/lib/query-session-channel";
import {
  formatQuerySessionError,
  isQuerySessionError,
} from "@/lib/query-session-error";
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

/**
 * Cap on the in-memory query history list. Mirrors the SQLite trim
 * in `src-tauri/src/storage.rs` — keep both in sync. Phase 1 bumped
 * this from 200 to 2000 so the dedicated Query History sub-tab can
 * surface more than just a day or two of activity.
 */
const QUERY_HISTORY_CAP = 2000;

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
  /** Exact SQL that produced the currently retained result for each tab. */
  queryExecutionSql: Record<string, string>;
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
  markQueryCancelling: (tabId: string) => void;
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
   * Tab-scoped cleanup — drops the tab's queryStatus, queryEdits, and
   * queryPreviews entries. Used by `retargetQueryTab` so results pinned
   * to the old connection don't leak into the retargeted view.
   */
  dropQueryStateForTab: (tabId: string) => void;

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
  queryExecutionSql: {},
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

  runQuery: async (tabId, options): Promise<QueryOutcome> => {
    const state = get();
    const tab = state.workspaceTabs.find((item) => item.id === tabId);
    if (!tab || tab.kind !== "query") {
      return { kind: "noop" };
    }
    if (state.queryStatus[tabId]) {
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
      let result: RunQueryResult;
      if (
        connectionAtRun?.engine === "PostgreSQL" &&
        querySessionChannelsAvailable()
      ) {
        const persistent = await get().executePersistentQuery(
          tabId,
          tab.connectionId,
          query,
        );
        if (persistent.kind === "cancelled") {
          set((current) => {
            const { [tabId]: _status, ...queryStatus } = current.queryStatus;
            return { queryStatus };
          });
          return { kind: "cancelled" };
        }
        const entry = buildEntry({
          status: "success",
          runtimeMs: persistent.runtimeMs,
          rowCount: persistent.rowCount,
        });
        set((s) => {
          const { [tabId]: _status, ...restStatus } = s.queryStatus;
          const { [tabId]: _stalePreview, ...restPreviews } = s.queryPreviews;
          return {
            queryPreviews: restPreviews,
            queryStatus: restStatus,
            queryExecutionSql: {
              ...s.queryExecutionSql,
              [tabId]: query,
            },
            queryHistory: [entry, ...s.queryHistory].slice(
              0,
              QUERY_HISTORY_CAP,
            ),
            workspaceTabs: s.workspaceTabs.map((item) =>
              item.id === tabId
                ? { ...item, lastRun: "Just now", isDirty: false }
                : item,
            ),
          };
        });
        get().applyConnectionActivity(tab.connectionId);
        await persistEntry(entry);
        return {
          kind: "completed",
          runtimeMs: persistent.runtimeMs,
          rowCount: persistent.rowCount,
          preview: null,
        };
      }
      result = await tauriInvoke<RunQueryResult>("run_query", {
        payload: { connectionId: tab.connectionId, query },
      });
      const entry = buildEntry({
        status: "success",
        runtimeMs: result.runtimeMs,
        rowCount: result.rowCount,
      });
      const preview: QueryPreviewData = {
        columns: result.columns,
        rows: result.rows,
        runtime: `${result.runtimeMs} ms`,
        rowCount: result.rowCount.toString(),
        cache: "Cold",
      };
      set((s) => {
        const { [tabId]: _status, ...restStatus } = s.queryStatus;
        return {
          queryPreviews: {
            ...s.queryPreviews,
            [tabId]: preview,
          },
          queryStatus: restStatus,
          queryExecutionSql: {
            ...s.queryExecutionSql,
            [tabId]: query,
          },
          queryHistory: [entry, ...s.queryHistory].slice(0, QUERY_HISTORY_CAP),
          workspaceTabs: s.workspaceTabs.map((item) =>
            item.id === tabId
              ? { ...item, lastRun: "Just now", isDirty: false }
              : item,
          ),
        };
      });
      // Owner-slice write — Connections slice updates the Connection
      // record. Lives outside the set above so each slice is
      // responsible for mutating its own state.
      get().applyConnectionActivity(tab.connectionId);
      await persistEntry(entry);
      return {
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowCount: result.rowCount,
        preview,
      };
    } catch (error) {
      const message = isQuerySessionError(error)
        ? formatQuerySessionError(error)
        : errorToMessage(error);
      console.error("Query failed", error, {
        kind:
          error && typeof error === "object" && "kind" in error
            ? error.kind
            : "unknown",
      });
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
          queryHistory: [entry, ...s.queryHistory].slice(0, QUERY_HISTORY_CAP),
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

  markQueryCancelling: (tabId) =>
    set((state) => ({
      queryStatus: { ...state.queryStatus, [tabId]: { state: "cancelling" } },
    })),

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
      const filterByTab = <T>(bag: Record<string, T>) => {
        const next: Record<string, T> = {};
        for (const [key, value] of Object.entries(bag)) {
          if (!tabIdSet.has(key)) {
            next[key] = value;
          }
        }
        return next;
      };
      return {
        queryStatus: filterByTab(state.queryStatus),
        queryEdits: filterByTab(state.queryEdits),
        queryPreviews: filterByTab(state.queryPreviews),
        queryExecutionSql: filterByTab(state.queryExecutionSql),
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

  dropQueryStateForTab: (tabId) =>
    set((state) => {
      const { [tabId]: _droppedStatus, ...restStatus } = state.queryStatus;
      const { [tabId]: _droppedEdits, ...restEdits } = state.queryEdits;
      const { [tabId]: _droppedPreview, ...nextPreviews } = state.queryPreviews;
      const { [tabId]: _droppedExecutionSql, ...nextExecutionSql } =
        state.queryExecutionSql;
      return {
        queryStatus: restStatus,
        queryEdits: restEdits,
        queryPreviews: nextPreviews,
        queryExecutionSql: nextExecutionSql,
      };
    }),
});
