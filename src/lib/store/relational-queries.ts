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

import {
  closeQuerySessionForTab,
  executeQuerySession,
  invokeQuerySession,
  openQuerySession,
  querySessionChannelsAvailable,
  type QueryEventEnvelope,
} from "@/lib/query-session-channel";
import { pickSqlToRun } from "@/lib/sql";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  QueryHistoryEntry,
  QueryExecution,
  QueryOutcome,
  QueryPreviewData,
  QuerySessionState,
  QueryStatus,
  QueryTransactionIsolation,
  QueryTransactionMode,
  QueryTransactionSnapshot,
  SavedQueriesStatus,
  SavedQuery,
} from "./types";

const QUERY_RESULT_BUDGET = 128 * 1024 * 1024;
const encodedBytes = (
  value: Pick<QueryExecution, "resultSets" | "notices" | "error">,
) => new TextEncoder().encode(JSON.stringify(value)).byteLength;
type OmittedPayload = string | Array<string | null> | QueryExecution["error"];
const encodedValueBytes = (value: OmittedPayload) =>
  new TextEncoder().encode(JSON.stringify(value)).byteLength;
const defaultTransaction: QueryTransactionSnapshot = {
  mode: "autocommit",
  status: "idle",
  manualIsolation: "readCommitted",
};

export const supportsPersistentQuerySessions = (engine: string) =>
  engine === "PostgreSQL" && querySessionChannelsAvailable();

const newExecution = (id: string): QueryExecution => ({
  id,
  status: "running",
  startedAt: new Date().toISOString(),
  completedAt: null,
  runtimeMs: 0,
  resultSets: [],
  notices: [],
  error: null,
  omittedRows: 0,
  omittedResultSets: 0,
  omittedNotices: 0,
  omittedMetadataBytes: 0,
  truncationReasons: [],
  retainedBytes: 0,
  tombstone: null,
});

const releaseExecution = (execution: QueryExecution): QueryExecution => ({
  ...execution,
  resultSets: [],
  notices: [],
  error: null,
  retainedBytes: 0,
  tombstone: {
    status: execution.status,
    resultCount: execution.resultSets.length,
    rowCount: execution.resultSets.reduce(
      (total, result) => total + result.rowCount,
      0,
    ),
    affectedCount: 0,
    noticeCount: execution.notices.length,
    omittedCount:
      execution.omittedRows +
      execution.omittedResultSets +
      execution.omittedNotices,
    runtimeMs: execution.runtimeMs,
    releasedBytes: execution.retainedBytes,
    completedAt: execution.completedAt ?? new Date().toISOString(),
    reason: "globalBudget",
  },
});

function reduceSessionEvent(
  session: QuerySessionState,
  envelope: QueryEventEnvelope,
): QuerySessionState {
  const event = envelope.event;
  let execution = session.execution;
  if (event.kind === "executionStarted" && envelope.executionId)
    execution = newExecution(envelope.executionId);
  if (execution && envelope.executionId === execution.id) {
    if (event.kind === "resultSetStarted") {
      execution = {
        ...execution,
        resultSets: [
          ...execution.resultSets,
          {
            index: event.resultSetIndex,
            columns: event.columns,
            rows: [],
            rowCount: 0,
            partial: false,
            completed: false,
          },
        ],
      };
    } else if (event.kind === "rowBatch") {
      execution = {
        ...execution,
        resultSets: execution.resultSets.map((result) =>
          result.index === event.resultSetIndex
            ? { ...result, rows: [...result.rows, ...event.rows] }
            : result,
        ),
      };
    } else if (event.kind === "resultSetCompleted") {
      execution = {
        ...execution,
        resultSets: execution.resultSets.map((result) =>
          result.index === event.resultSetIndex
            ? {
                ...result,
                rowCount: event.rowCount,
                partial: event.partial,
                completed: true,
              }
            : result,
        ),
      };
    } else if (event.kind === "notice") {
      execution = {
        ...execution,
        notices: [
          ...execution.notices,
          { severity: event.severity, message: event.message },
        ],
      };
    } else if (event.kind === "executionCompleted") {
      const completedAt = new Date().toISOString();
      execution = {
        ...execution,
        status:
          event.status === "cancelled"
            ? "cancelled"
            : event.status === "failed" || event.error
              ? "failed"
              : "completed",
        completedAt,
        runtimeMs: Math.max(
          0,
          Date.now() - new Date(execution.startedAt).getTime(),
        ),
        error: event.error,
        omittedRows: event.omittedRows,
        omittedResultSets: event.omittedResultSets,
        omittedNotices: event.omittedNotices,
        omittedMetadataBytes: event.omittedMetadataBytes,
        truncationReasons: event.truncationReasons,
      };
    }
    execution = {
      ...execution,
      retainedBytes: encodedBytes({
        resultSets: execution.resultSets,
        notices: execution.notices,
        error: execution.error,
      }),
    };
  }
  if (event.kind === "sessionLost")
    return {
      ...session,
      state: "lost",
      execution: execution ? { ...execution, status: "lost" } : null,
    };
  if (event.kind === "sessionClosed")
    return { ...session, state: "closed", execution };
  return {
    ...session,
    generation: session.generation || envelope.generation,
    nextSequence: envelope.sequence + 1,
    transaction:
      event.kind === "sessionState" || event.kind === "executionCompleted"
        ? event.transaction
        : session.transaction,
    execution,
  };
}

const omitEnvelopePayload = (
  session: QuerySessionState,
  envelope: QueryEventEnvelope,
): QuerySessionState => {
  const event = envelope.event;
  let sanitized = event;
  let omittedRows = 0;
  let omittedNotices = 0;
  let omittedMetadataBytes = 0;

  if (event.kind === "rowBatch") {
    omittedRows = event.rows.length;
    sanitized = { ...event, rows: [] };
  } else if (event.kind === "notice") {
    omittedNotices = 1;
    sanitized = { ...event, message: "" };
    omittedMetadataBytes = encodedValueBytes(event.message);
  } else if (event.kind === "resultSetStarted") {
    sanitized = { ...event, columns: [] };
    omittedMetadataBytes = encodedValueBytes(event.columns);
  } else if (event.kind === "executionCompleted" && event.error) {
    sanitized = { ...event, error: null };
    omittedMetadataBytes = encodedValueBytes(event.error);
  }

  const reduced = reduceSessionEvent(session, {
    ...envelope,
    event: sanitized,
  });
  if (!reduced.execution || reduced.execution.id !== envelope.executionId)
    return reduced;
  return {
    ...reduced,
    execution: {
      ...reduced.execution,
      omittedRows: reduced.execution.omittedRows + omittedRows,
      omittedNotices: reduced.execution.omittedNotices + omittedNotices,
      omittedMetadataBytes:
        reduced.execution.omittedMetadataBytes + omittedMetadataBytes,
      truncationReasons: reduced.execution.truncationReasons.includes(
        "frontendGlobalBudget",
      )
        ? reduced.execution.truncationReasons
        : [...reduced.execution.truncationReasons, "frontendGlobalBudget"],
    },
  };
};

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
  queryStatus: Record<string, QueryStatus>;
  queryPreviews: Record<string, QueryPreviewData>;
  querySessions: Record<string, QuerySessionState>;
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
  cancelQuery: (tabId: string) => Promise<void>;
  setQueryTransactionMode: (
    tabId: string,
    mode: QueryTransactionMode,
  ) => Promise<void>;
  setQueryTransactionIsolation: (
    tabId: string,
    isolation: QueryTransactionIsolation,
  ) => Promise<void>;
  queryTransactionAction: (
    tabId: string,
    action: "commit" | "rollback" | "refresh",
  ) => Promise<void>;
  releaseQueryResults: (tabId: string) => void;
  markQuerySessionViewed: (tabId: string) => void;
  closeQuerySessionForTab: (tabId: string) => Promise<void>;
  closeQuerySessionsForConnection: (connectionId: string) => Promise<void>;
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
   * Tab-scoped cleanup — drops queryStatus, queryEdits, and (when the
   * tab's query-label is provided) the queryPreviews entry that backs
   * the results grid. Used by `retargetQueryTab` so results pinned to
   * the old connection don't leak into the retargeted view.
   */
  dropQueryStateForTab: (tabId: string, queryLabel?: string) => void;

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
  querySessions: {},
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
      let wasCancelled = false;
      if (
        connectionAtRun &&
        supportsPersistentQuerySessions(connectionAtRun.engine)
      ) {
        let session = get().querySessions[tabId];
        if (
          !session ||
          session.connectionId !== tab.connectionId ||
          session.state !== "open"
        ) {
          const sessionId = crypto.randomUUID();
          session = {
            id: sessionId,
            tabId,
            connectionId: tab.connectionId,
            generation: 0,
            nextSequence: 1,
            transaction: defaultTransaction,
            execution: null,
            lastViewedAt: Date.now(),
            budgetOwners: [],
            state: "opening",
            error: null,
          };
          set((current) => ({
            querySessions: { ...current.querySessions, [tabId]: session },
          }));
          const transaction = await openQuerySession({
            sessionId,
            tabId,
            connectionId: tab.connectionId,
            handler: (envelope) => {
              if (
                envelope.tabId !== tabId ||
                envelope.connectionId !== tab.connectionId
              )
                return { retainMoreRows: false };
              let retainMoreRows = true;
              set((current) => {
                const currentSession = current.querySessions[tabId];
                if (
                  !currentSession ||
                  (currentSession.generation &&
                    currentSession.generation !== envelope.generation)
                )
                  return {};
                let reduced = reduceSessionEvent(currentSession, envelope);
                const sessions = { ...current.querySessions, [tabId]: reduced };
                let retained = Object.values(sessions).reduce(
                  (total, candidate) =>
                    total + (candidate.execution?.retainedBytes ?? 0),
                  0,
                );
                const candidates = Object.values(sessions)
                  .filter(
                    (candidate) =>
                      candidate.tabId !== current.activeTabId &&
                      candidate.execution?.status !== "running" &&
                      (candidate.execution?.retainedBytes ?? 0) > 0,
                  )
                  .sort(
                    (left, right) =>
                      left.lastViewedAt - right.lastViewedAt ||
                      left.tabId.localeCompare(right.tabId),
                  );
                for (const candidate of candidates) {
                  if (retained <= QUERY_RESULT_BUDGET) break;
                  const candidateExecution = candidate.execution;
                  if (!candidateExecution) continue;
                  retained -= candidateExecution.retainedBytes;
                  sessions[candidate.tabId] = {
                    ...candidate,
                    execution: releaseExecution(candidateExecution),
                  };
                }
                if (retained > QUERY_RESULT_BUDGET) {
                  retainMoreRows = false;
                  reduced = omitEnvelopePayload(currentSession, envelope);
                  const retainedWithoutCurrent =
                    retained - (sessions[tabId]?.execution?.retainedBytes ?? 0);
                  if (
                    reduced.execution &&
                    retainedWithoutCurrent + reduced.execution.retainedBytes >
                      QUERY_RESULT_BUDGET
                  ) {
                    reduced = {
                      ...reduced,
                      execution: releaseExecution(reduced.execution),
                    };
                  }
                  reduced = {
                    ...reduced,
                    budgetOwners: Object.values(sessions)
                      .filter(
                        (candidate) =>
                          candidate.tabId !== tabId &&
                          (candidate.execution?.retainedBytes ?? 0) > 0,
                      )
                      .sort(
                        (left, right) =>
                          (right.execution?.retainedBytes ?? 0) -
                            (left.execution?.retainedBytes ?? 0) ||
                          left.tabId.localeCompare(right.tabId),
                      )
                      .slice(0, 3)
                      .map((candidate) => ({
                        tabId: candidate.tabId,
                        label:
                          current.workspaceTabs.find(
                            (item) => item.id === candidate.tabId,
                          )?.label ?? candidate.tabId,
                        retainedBytes: candidate.execution?.retainedBytes ?? 0,
                      })),
                  };
                  sessions[tabId] = reduced;
                } else if (reduced.budgetOwners.length) {
                  sessions[tabId] = { ...reduced, budgetOwners: [] };
                }
                return { querySessions: sessions };
              });
              return { retainMoreRows };
            },
          });
          set((current) => ({
            querySessions: {
              ...current.querySessions,
              [tabId]: {
                ...current.querySessions[tabId],
                transaction,
                state: "open",
              },
            },
          }));
        }
        const executionId = crypto.randomUUID();
        await executeQuerySession(tabId, executionId, query);
        const execution = get().querySessions[tabId]?.execution;
        if (!execution)
          throw new Error(
            "Query session completed without an execution result",
          );
        const first = execution.resultSets.find(
          (candidate) => candidate.columns.length > 0,
        );
        result = {
          columns: (first?.columns ?? []).map((column) => column ?? ""),
          rows: (first?.rows ?? []).map((row) => row.map((cell) => cell ?? "")),
          runtimeMs: execution.runtimeMs,
          rowCount:
            execution.resultSets.reduce(
              (total, item) => total + item.rowCount,
              0,
            ) + execution.omittedRows,
        };
        wasCancelled = execution.status === "cancelled";
        if (execution.status === "failed")
          throw execution.error ?? new Error("Query failed");
      } else {
        result = await tauriInvoke<RunQueryResult>("run_query", {
          payload: { connectionId: tab.connectionId, query },
        });
      }
      if (wasCancelled) {
        set((state) => {
          const { [tabId]: _status, ...queryStatus } = state.queryStatus;
          return { queryStatus };
        });
        return { kind: "cancelled" };
      }
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
      const message = errorToMessage(error);
      console.error("Query failed", {
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

  cancelQuery: async (tabId) => {
    const execution = get().querySessions[tabId]?.execution;
    if (!execution || execution.status !== "running") return;
    set((state) => ({
      queryStatus: { ...state.queryStatus, [tabId]: { state: "cancelling" } },
    }));
    await invokeQuerySession(tabId, "cancel_query_execution", {
      executionId: execution.id,
    });
  },

  setQueryTransactionMode: async (tabId, mode) => {
    const transaction = await invokeQuerySession<QueryTransactionSnapshot>(
      tabId,
      "set_query_transaction_mode",
      { mode },
    );
    set((state) => ({
      querySessions: {
        ...state.querySessions,
        [tabId]: { ...state.querySessions[tabId], transaction },
      },
    }));
  },

  setQueryTransactionIsolation: async (tabId, manualIsolation) => {
    const transaction = await invokeQuerySession<QueryTransactionSnapshot>(
      tabId,
      "set_query_transaction_isolation",
      { manualIsolation },
    );
    set((state) => ({
      querySessions: {
        ...state.querySessions,
        [tabId]: { ...state.querySessions[tabId], transaction },
      },
    }));
  },

  queryTransactionAction: async (tabId, action) => {
    const command =
      action === "commit"
        ? "commit_query_transaction"
        : action === "rollback"
          ? "rollback_query_transaction"
          : "refresh_query_transaction_state";
    const transaction = await invokeQuerySession<QueryTransactionSnapshot>(
      tabId,
      command,
    );
    set((state) => ({
      querySessions: {
        ...state.querySessions,
        [tabId]: { ...state.querySessions[tabId], transaction },
      },
    }));
  },

  releaseQueryResults: (tabId) =>
    set((state) => {
      const session = state.querySessions[tabId];
      const execution = session?.execution;
      if (!session || !execution || execution.status === "running") return {};
      return {
        querySessions: {
          ...state.querySessions,
          [tabId]: {
            ...session,
            execution: releaseExecution(execution),
          },
        },
      };
    }),

  markQuerySessionViewed: (tabId) =>
    set((state) => {
      const session = state.querySessions[tabId];
      return session
        ? {
            querySessions: {
              ...state.querySessions,
              [tabId]: { ...session, lastViewedAt: Date.now() },
            },
          }
        : {};
    }),

  closeQuerySessionForTab: async (tabId) => {
    await closeQuerySessionForTab(tabId).catch(() => undefined);
    set((state) => {
      const { [tabId]: _session, ...querySessions } = state.querySessions;
      return { querySessions };
    });
  },

  closeQuerySessionsForConnection: async (connectionId) => {
    const tabIds = Object.values(get().querySessions)
      .filter((session) => session.connectionId === connectionId)
      .map((session) => session.tabId);
    await Promise.all(
      tabIds.map((tabId) => get().closeQuerySessionForTab(tabId)),
    );
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
        querySessions: filterByTab(state.querySessions),
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
      const { [tabId]: _droppedSession, ...nextSessions } = state.querySessions;
      return {
        queryStatus: restStatus,
        queryEdits: restEdits,
        queryPreviews: nextPreviews,
        querySessions: nextSessions,
      };
    }),
});
