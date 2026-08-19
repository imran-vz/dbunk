/**
 * Persistent Query Sessions slice — owns PostgreSQL editor-session state,
 * event reduction, retained-result budgeting, and transaction commands.
 *
 * Relational Queries decides whether a query should use a persistent session;
 * this slice owns everything after that decision.
 */

import type { StateCreator } from "zustand";

import {
  cancelQueryExecution,
  closeQuerySessionForTab as closeChannelForTab,
  commitQueryTransaction,
  executeQuerySession,
  openQuerySession,
  refreshQueryTransaction,
  rollbackQueryTransaction,
  setQuerySessionTransactionIsolation,
  setQuerySessionTransactionMode,
  type QueryEventEnvelope,
} from "@/lib/query-session-channel";

import type {
  AppStoreState,
  QueryExecution,
  QuerySessionState,
  QueryTransactionIsolation,
  QueryTransactionMode,
  QueryTransactionSnapshot,
} from "./types";

const QUERY_RESULT_BUDGET = 128 * 1024 * 1024;
const defaultTransaction: QueryTransactionSnapshot = {
  mode: "autocommit",
  status: "idle",
  manualIsolation: "readCommitted",
};

const encodedBytes = (
  value: Pick<QueryExecution, "resultSets" | "notices" | "error">,
) => new TextEncoder().encode(JSON.stringify(value)).byteLength;

type OmittedPayload = string | Array<string | null> | QueryExecution["error"];
const encodedValueBytes = (value: OmittedPayload) =>
  new TextEncoder().encode(JSON.stringify(value)).byteLength;

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

const reduceExecutionEvent = (
  execution: QueryExecution,
  envelope: QueryEventEnvelope,
): QueryExecution => {
  const event = envelope.event;
  let next = execution;
  if (event.kind === "resultSetStarted") {
    next = {
      ...next,
      resultSets: [
        ...next.resultSets,
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
    next = {
      ...next,
      resultSets: next.resultSets.map((result) =>
        result.index === event.resultSetIndex
          ? { ...result, rows: [...result.rows, ...event.rows] }
          : result,
      ),
    };
  } else if (event.kind === "resultSetCompleted") {
    next = {
      ...next,
      resultSets: next.resultSets.map((result) =>
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
    next = {
      ...next,
      notices: [
        ...next.notices,
        { severity: event.severity, message: event.message },
      ],
    };
  } else if (event.kind === "executionCompleted") {
    next = {
      ...next,
      status:
        event.status === "cancelled"
          ? "cancelled"
          : event.error
            ? "failed"
            : event.status,
      completedAt: new Date().toISOString(),
      runtimeMs: Math.max(0, Date.now() - new Date(next.startedAt).getTime()),
      error: event.error,
      omittedRows: event.omittedRows,
      omittedResultSets: event.omittedResultSets,
      omittedNotices: event.omittedNotices,
      omittedMetadataBytes: event.omittedMetadataBytes,
      truncationReasons: event.truncationReasons,
    };
  }
  return {
    ...next,
    retainedBytes: encodedBytes({
      resultSets: next.resultSets,
      notices: next.notices,
      error: next.error,
    }),
  };
};

const reduceSessionEvent = (
  session: QuerySessionState,
  envelope: QueryEventEnvelope,
): QuerySessionState => {
  const event = envelope.event;
  const started =
    event.kind === "executionStarted" && envelope.executionId
      ? newExecution(envelope.executionId)
      : session.execution;
  const execution =
    started && envelope.executionId === started.id
      ? reduceExecutionEvent(started, envelope)
      : started;

  if (event.kind === "sessionLost") {
    return {
      ...session,
      state: "lost",
      execution: execution ? { ...execution, status: "lost" } : null,
    };
  }
  if (event.kind === "sessionClosed") {
    return { ...session, state: "closed", execution };
  }
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
};

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
  if (!reduced.execution || reduced.execution.id !== envelope.executionId) {
    return reduced;
  }
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

const retainedBytes = (sessions: Record<string, QuerySessionState>) =>
  Object.values(sessions).reduce(
    (total, session) => total + (session.execution?.retainedBytes ?? 0),
    0,
  );

const evictInactiveResults = (
  sessions: Record<string, QuerySessionState>,
  activeTabId: string,
): number => {
  let retained = retainedBytes(sessions);
  const candidates = Object.values(sessions)
    .filter(
      (session) =>
        session.tabId !== activeTabId &&
        session.execution?.status !== "running" &&
        (session.execution?.retainedBytes ?? 0) > 0,
    )
    .sort(
      (left, right) =>
        left.lastViewedAt - right.lastViewedAt ||
        left.tabId.localeCompare(right.tabId),
    );
  for (const session of candidates) {
    if (retained <= QUERY_RESULT_BUDGET) break;
    if (!session.execution) continue;
    retained -= session.execution.retainedBytes;
    sessions[session.tabId] = {
      ...session,
      execution: releaseExecution(session.execution),
    };
  }
  return retained;
};

const describeBudgetOwners = (
  state: AppStoreState,
  sessions: Record<string, QuerySessionState>,
  tabId: string,
) =>
  Object.values(sessions)
    .filter(
      (session) =>
        session.tabId !== tabId && (session.execution?.retainedBytes ?? 0) > 0,
    )
    .sort(
      (left, right) =>
        (right.execution?.retainedBytes ?? 0) -
          (left.execution?.retainedBytes ?? 0) ||
        left.tabId.localeCompare(right.tabId),
    )
    .slice(0, 3)
    .map((session) => ({
      tabId: session.tabId,
      label:
        state.workspaceTabs.find((tab) => tab.id === session.tabId)?.label ??
        session.tabId,
      retainedBytes: session.execution?.retainedBytes ?? 0,
    }));

type EventBudgetResult = {
  querySessions: Record<string, QuerySessionState>;
  retainMoreRows: boolean;
};

const applyEventBudget = (
  state: AppStoreState,
  tabId: string,
  envelope: QueryEventEnvelope,
): EventBudgetResult => {
  const currentSession = state.querySessions[tabId];
  if (
    !currentSession ||
    (currentSession.generation &&
      currentSession.generation !== envelope.generation)
  ) {
    return { querySessions: state.querySessions, retainMoreRows: false };
  }

  let current = reduceSessionEvent(currentSession, envelope);
  const sessions = { ...state.querySessions, [tabId]: current };
  const retained = evictInactiveResults(sessions, state.activeTabId);
  if (retained <= QUERY_RESULT_BUDGET) {
    if (current.budgetOwners.length) {
      sessions[tabId] = { ...current, budgetOwners: [] };
    }
    return { querySessions: sessions, retainMoreRows: true };
  }

  current = omitEnvelopePayload(currentSession, envelope);
  const retainedWithoutCurrent =
    retained - (sessions[tabId]?.execution?.retainedBytes ?? 0);
  if (
    current.execution &&
    retainedWithoutCurrent + current.execution.retainedBytes >
      QUERY_RESULT_BUDGET
  ) {
    current = { ...current, execution: releaseExecution(current.execution) };
  }
  sessions[tabId] = {
    ...current,
    budgetOwners: describeBudgetOwners(state, sessions, tabId),
  };
  return { querySessions: sessions, retainMoreRows: false };
};

export type PersistentQueryResult =
  | {
      kind: "completed";
      columns: string[];
      rows: string[][];
      runtimeMs: number;
      rowCount: number;
    }
  | { kind: "cancelled" };

export type QuerySessionsSlice = {
  querySessions: Record<string, QuerySessionState>;
  executePersistentQuery: (
    tabId: string,
    connectionId: string,
    sql: string,
  ) => Promise<PersistentQueryResult>;
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
};

const transactionActions = {
  commit: commitQueryTransaction,
  rollback: rollbackQueryTransaction,
  refresh: refreshQueryTransaction,
} satisfies Record<
  "commit" | "rollback" | "refresh",
  (tabId: string) => Promise<QueryTransactionSnapshot>
>;

export const createQuerySessionsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  QuerySessionsSlice
> = (set, get) => ({
  querySessions: {},

  executePersistentQuery: async (tabId, connectionId, sql) => {
    const existing = get().querySessions[tabId];
    if (
      !existing ||
      existing.connectionId !== connectionId ||
      existing.state !== "open"
    ) {
      const session: QuerySessionState = {
        id: crypto.randomUUID(),
        tabId,
        connectionId,
        generation: 0,
        nextSequence: 1,
        transaction: defaultTransaction,
        execution: null,
        lastViewedAt: Date.now(),
        budgetOwners: [],
        state: "opening",
        error: null,
      };
      set((state) => ({
        querySessions: { ...state.querySessions, [tabId]: session },
      }));
      const transaction = await openQuerySession({
        sessionId: session.id,
        tabId,
        connectionId,
        handler: (envelope) => {
          if (
            envelope.tabId !== tabId ||
            envelope.connectionId !== connectionId
          ) {
            return { retainMoreRows: false };
          }
          let retainMoreRows = false;
          set((state) => {
            const budgeted = applyEventBudget(state, tabId, envelope);
            retainMoreRows = budgeted.retainMoreRows;
            return budgeted.querySessions === state.querySessions
              ? {}
              : { querySessions: budgeted.querySessions };
          });
          return { retainMoreRows };
        },
      });
      set((state) => {
        const current = state.querySessions[tabId];
        return current
          ? {
              querySessions: {
                ...state.querySessions,
                [tabId]: { ...current, transaction, state: "open" },
              },
            }
          : {};
      });
    }

    await executeQuerySession(tabId, crypto.randomUUID(), sql);
    const execution = get().querySessions[tabId]?.execution;
    if (!execution) {
      throw new Error("Query session completed without an execution result");
    }
    if (execution.status === "cancelled") return { kind: "cancelled" };
    if (execution.status === "failed") {
      throw execution.error ?? new Error("Query failed");
    }
    const first = execution.resultSets.find(
      (result) => result.columns.length > 0,
    );
    return {
      kind: "completed",
      columns: (first?.columns ?? []).map((column) => column ?? ""),
      rows: (first?.rows ?? []).map((row) => row.map((cell) => cell ?? "")),
      runtimeMs: execution.runtimeMs,
      rowCount:
        execution.resultSets.reduce(
          (total, result) => total + result.rowCount,
          0,
        ) + execution.omittedRows,
    };
  },

  cancelQuery: async (tabId) => {
    const execution = get().querySessions[tabId]?.execution;
    if (!execution || execution.status !== "running") return;
    set((state) => ({
      queryStatus: { ...state.queryStatus, [tabId]: { state: "cancelling" } },
    }));
    await cancelQueryExecution(tabId, execution.id);
  },

  setQueryTransactionMode: async (tabId, mode) => {
    const transaction = await setQuerySessionTransactionMode(tabId, mode);
    set((state) => {
      const session = state.querySessions[tabId];
      return session
        ? {
            querySessions: {
              ...state.querySessions,
              [tabId]: { ...session, transaction },
            },
          }
        : {};
    });
  },

  setQueryTransactionIsolation: async (tabId, isolation) => {
    const transaction = await setQuerySessionTransactionIsolation(
      tabId,
      isolation,
    );
    set((state) => {
      const session = state.querySessions[tabId];
      return session
        ? {
            querySessions: {
              ...state.querySessions,
              [tabId]: { ...session, transaction },
            },
          }
        : {};
    });
  },

  queryTransactionAction: async (tabId, action) => {
    const transaction = await transactionActions[action](tabId);
    set((state) => {
      const session = state.querySessions[tabId];
      return session
        ? {
            querySessions: {
              ...state.querySessions,
              [tabId]: { ...session, transaction },
            },
          }
        : {};
    });
  },

  releaseQueryResults: (tabId) =>
    set((state) => {
      const session = state.querySessions[tabId];
      const execution = session?.execution;
      if (!session || !execution || execution.status === "running") return {};
      return {
        querySessions: {
          ...state.querySessions,
          [tabId]: { ...session, execution: releaseExecution(execution) },
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
    await closeChannelForTab(tabId).catch(() => undefined);
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
});
