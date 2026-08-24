import type {
  QueryEventEnvelope,
  QuerySessionEvent,
} from "@/lib/query-session-channel";
import type {
  QueryDatabaseError,
  QueryExecution,
  QueryNotice,
  QueryResultSet,
  QuerySessionState,
  QueryTransactionSnapshot,
} from "@/lib/store/types";

export const QUERY_RESULT_BUDGET = 128 * 1024 * 1024;

export const defaultTransaction: QueryTransactionSnapshot = {
  mode: "autocommit",
  status: "idle",
  manualIsolation: "readCommitted",
};

const textEncoder = new TextEncoder();

type EncodedPayload =
  | string
  | Array<string | null>
  | Array<Array<string | null>>
  | QueryNotice
  | QueryDatabaseError;

export const encodedValueBytes = (value: EncodedPayload) =>
  textEncoder.encode(JSON.stringify(value)).byteLength;

export function flattenResultSetRows(
  result: QueryResultSet | undefined,
): Array<Array<string | null>> {
  if (!result || result.rowChunks.length === 0) return [];
  if (result.rowChunks.length === 1) return result.rowChunks[0] ?? [];
  return result.rowChunks.flat();
}

export const newExecution = (id: string): QueryExecution => ({
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

export const releaseExecution = (
  execution: QueryExecution,
): QueryExecution => ({
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

const payloadBytes = (event: QuerySessionEvent): number => {
  switch (event.kind) {
    case "resultSetStarted":
      return encodedValueBytes(event.columns);
    case "rowBatch":
      return encodedValueBytes(event.rows);
    case "notice":
      return encodedValueBytes({
        severity: event.severity,
        message: event.message,
      });
    case "executionCompleted":
      return event.error ? encodedValueBytes(event.error) : 0;
    default:
      return 0;
  }
};

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
          rowChunks: [],
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
          ? { ...result, rowChunks: [...result.rowChunks, event.rows] }
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
    retainedBytes: execution.retainedBytes + payloadBytes(event),
  };
};

export const reduceSessionEvent = (
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
    generation: session.generation ?? envelope.generation,
    transaction:
      event.kind === "sessionState" || event.kind === "executionCompleted"
        ? event.transaction
        : session.transaction,
    execution,
  };
};

export const omitEnvelopePayload = (
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

export const retainedBytes = (sessions: Record<string, QuerySessionState>) =>
  Object.values(sessions).reduce(
    (total, session) => total + (session.execution?.retainedBytes ?? 0),
    0,
  );

export const evictInactiveResults = (
  sessions: Record<string, QuerySessionState>,
  activeTabId: string,
  budget = QUERY_RESULT_BUDGET,
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
    if (retained <= budget) break;
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
  state: QuerySessionBudgetState,
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

export type QuerySessionBudgetState = {
  querySessions: Record<string, QuerySessionState>;
  activeTabId: string;
  workspaceTabs: Array<{ id: string; label: string }>;
};

export type EventBudgetResult = {
  querySessions: Record<string, QuerySessionState>;
  retainMoreRows: boolean;
};

export const applyEventBudget = (
  state: QuerySessionBudgetState,
  tabId: string,
  envelope: QueryEventEnvelope,
  budget = QUERY_RESULT_BUDGET,
): EventBudgetResult => {
  const currentSession = state.querySessions[tabId];
  if (
    !currentSession ||
    (currentSession.generation !== null &&
      currentSession.generation !== envelope.generation)
  ) {
    return { querySessions: state.querySessions, retainMoreRows: false };
  }

  let current = reduceSessionEvent(currentSession, envelope);
  const sessions = { ...state.querySessions, [tabId]: current };
  const retained = evictInactiveResults(sessions, state.activeTabId, budget);
  if (retained <= budget) {
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
    retainedWithoutCurrent + current.execution.retainedBytes > budget
  ) {
    current = { ...current, execution: releaseExecution(current.execution) };
  }
  sessions[tabId] = {
    ...current,
    budgetOwners: describeBudgetOwners(state, sessions, tabId),
  };
  return { querySessions: sessions, retainMoreRows: false };
};
