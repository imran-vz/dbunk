/* oxlint-disable anti-slop/no-runtime-typeof -- Channel availability is a window-global check; `__TAURI_INTERNALS__` is required for Tauri v2 Channels and is not interchangeable with `isTauri()`. */
import { Channel } from "@tauri-apps/api/core";

import type {
  QueryDatabaseError,
  QueryExecutionTerminalStatus,
  QuerySessionError,
  QueryTransactionSnapshot,
} from "@/lib/store/types";
import { tauriInvoke } from "@/lib/tauri";

export type QuerySessionEvent =
  | { kind: "sessionState"; transaction: QueryTransactionSnapshot }
  | { kind: "executionStarted" }
  | {
      kind: "resultSetStarted";
      resultSetIndex: number;
      columns: Array<string | null>;
    }
  | {
      kind: "rowBatch";
      resultSetIndex: number;
      rows: Array<Array<string | null>>;
    }
  | {
      kind: "resultSetCompleted";
      resultSetIndex: number;
      rowCount: number;
      partial: boolean;
    }
  | { kind: "notice"; severity: string; message: string }
  | {
      kind: "executionCompleted";
      status: QueryExecutionTerminalStatus;
      transaction: QueryTransactionSnapshot;
      omittedRows: number;
      omittedResultSets: number;
      omittedNotices: number;
      omittedMetadataBytes: number;
      truncationReasons: string[];
      error: QueryDatabaseError | null;
    }
  | { kind: "sessionLost"; reason: string }
  | { kind: "sessionClosed" };

export type QueryEventEnvelope = {
  sessionId: string;
  tabId: string;
  connectionId: string;
  generation: number;
  sequence: number;
  executionId: string | null;
  requiresAck: boolean;
  event: QuerySessionEvent;
};

type Handler = (envelope: QueryEventEnvelope) => { retainMoreRows: boolean };
type PendingRun = {
  executionId: string;
  resolve: () => void;
  reject: (error: QuerySessionError) => void;
};
type SessionBinding = {
  sessionId: string;
  tabId: string;
  handler: Handler;
  nextSequence: number;
  pendingAck: {
    executionId: string;
    sequence: number;
    retainMoreRows: boolean;
    terminal: boolean;
  } | null;
  ackInFlight: boolean;
  pendingRun: PendingRun | null;
};

const ownerId = crypto.randomUUID();
const bindings = new Map<string, SessionBinding>();
let registration: Promise<void> | null = null;
let heartbeatTimer: ReturnType<typeof setInterval> | null = null;
let onFocusHeartbeat: (() => void) | null = null;
let onVisibilityHeartbeat: (() => void) | null = null;

export const querySessionChannelsAvailable = () =>
  // Tauri v2 Channels bind through `__TAURI_INTERNALS__` specifically;
  // `isTauri()` also treats legacy `__TAURI__` as native and is not reused.
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const heartbeat = async () => {
  if (!bindings.size) return;
  await tauriInvoke("heartbeat_query_sessions", {
    payload: {
      ownerId,
      sessionIds: [...bindings.values()].map((binding) => binding.sessionId),
    },
  });
};

const ensureHeartbeat = () => {
  if (heartbeatTimer) return;
  heartbeatTimer = setInterval(
    () => void heartbeat().catch(() => undefined),
    30_000,
  );
  onFocusHeartbeat = () => void heartbeat().catch(() => undefined);
  onVisibilityHeartbeat = () => {
    if (document.visibilityState === "visible") onFocusHeartbeat?.();
  };
  window.addEventListener("focus", onFocusHeartbeat);
  document.addEventListener("visibilitychange", onVisibilityHeartbeat);
};

const register = async () => {
  registration ??= tauriInvoke("register_query_session_owner", {
    payload: { ownerId },
  }).then(() => undefined);
  await registration;
  ensureHeartbeat();
};

const flushAck = async (binding: SessionBinding) => {
  if (binding.ackInFlight) return;
  binding.ackInFlight = true;
  try {
    while (binding.pendingAck) {
      const ack = binding.pendingAck;
      binding.pendingAck = null;
      try {
        await tauriInvoke("ack_query_session_events", {
          payload: {
            sessionId: binding.sessionId,
            executionId: ack.executionId,
            ackThroughSequence: ack.sequence,
            retainMoreRows: ack.retainMoreRows,
          },
        });
        if (
          ack.terminal &&
          binding.pendingRun?.executionId === ack.executionId
        ) {
          const pending = binding.pendingRun;
          binding.pendingRun = null;
          pending.resolve();
        }
      } catch {
        binding.pendingRun?.reject({ kind: "connectionLost" });
        binding.pendingRun = null;
        break;
      }
    }
  } finally {
    binding.ackInFlight = false;
    if (binding.pendingAck) {
      void flushAck(binding);
    }
  }
};

const receive = (binding: SessionBinding, envelope: QueryEventEnvelope) => {
  if (
    envelope.sessionId !== binding.sessionId ||
    envelope.sequence < binding.nextSequence
  )
    return;
  if (envelope.sequence !== binding.nextSequence) {
    binding.pendingRun?.reject({ kind: "invalidSequence" });
    binding.pendingRun = null;
    bindings.delete(binding.tabId);
    binding.handler({
      ...envelope,
      requiresAck: false,
      event: { kind: "sessionLost", reason: "invalidSequence" },
    });
    void closeQuerySession(binding.sessionId);
    return;
  }
  binding.nextSequence += 1;
  const { retainMoreRows } = binding.handler(envelope);
  if (
    envelope.event.kind === "sessionLost" ||
    envelope.event.kind === "sessionClosed"
  ) {
    binding.pendingRun?.reject({
      kind:
        envelope.event.kind === "sessionLost"
          ? "connectionLost"
          : "sessionNotFound",
    });
    binding.pendingRun = null;
    return;
  }
  if (!envelope.requiresAck || !envelope.executionId) return;
  binding.pendingAck = {
    executionId: envelope.executionId,
    sequence: envelope.sequence,
    retainMoreRows:
      (binding.pendingAck?.retainMoreRows ?? true) && retainMoreRows,
    terminal:
      (binding.pendingAck?.terminal ?? false) ||
      envelope.event.kind === "executionCompleted",
  };
  void flushAck(binding);
};

export async function openQuerySession(input: {
  sessionId: string;
  tabId: string;
  connectionId: string;
  handler: Handler;
}): Promise<QueryTransactionSnapshot> {
  await register();
  const binding: SessionBinding = {
    sessionId: input.sessionId,
    tabId: input.tabId,
    handler: input.handler,
    nextSequence: 1,
    pendingAck: null,
    ackInFlight: false,
    pendingRun: null,
  };
  const onEvent = new Channel<QueryEventEnvelope>();
  onEvent.onmessage = (event) => receive(binding, event);
  bindings.set(input.tabId, binding);
  try {
    return await tauriInvoke("open_query_session", {
      payload: {
        ownerId,
        sessionId: input.sessionId,
        tabId: input.tabId,
        connectionId: input.connectionId,
      },
      onEvent,
    });
  } catch (error) {
    bindings.delete(input.tabId);
    throw error;
  }
}

export async function executeQuerySession(
  tabId: string,
  executionId: string,
  sql: string,
) {
  const binding = bindings.get(tabId);
  if (!binding) throw { kind: "sessionNotFound" } satisfies QuerySessionError;
  const settled = new Promise<void>((resolve, reject) => {
    binding.pendingRun = { executionId, resolve, reject };
  });
  try {
    await tauriInvoke("execute_query_session", {
      payload: { sessionId: binding.sessionId, executionId, sql },
    });
  } catch (error) {
    binding.pendingRun = null;
    throw error;
  }
  return settled;
}

const querySessionIdForTab = (tabId: string) =>
  bindings.get(tabId)?.sessionId ?? null;

const sessionIdForTab = (tabId: string): string => {
  const sessionId = querySessionIdForTab(tabId);
  if (!sessionId) throw { kind: "sessionNotFound" } satisfies QuerySessionError;
  return sessionId;
};

export async function cancelQueryExecution(
  tabId: string,
  executionId: string,
): Promise<void> {
  await tauriInvoke("cancel_query_execution", {
    payload: { sessionId: sessionIdForTab(tabId), executionId },
  });
}

export const setQuerySessionTransactionMode = (
  tabId: string,
  mode: QueryTransactionSnapshot["mode"],
) =>
  tauriInvoke<QueryTransactionSnapshot>("set_query_transaction_mode", {
    payload: { sessionId: sessionIdForTab(tabId), mode },
  });

export const setQuerySessionTransactionIsolation = (
  tabId: string,
  manualIsolation: QueryTransactionSnapshot["manualIsolation"],
) =>
  tauriInvoke<QueryTransactionSnapshot>("set_query_transaction_isolation", {
    payload: { sessionId: sessionIdForTab(tabId), manualIsolation },
  });

export const commitQueryTransaction = (tabId: string) =>
  tauriInvoke<QueryTransactionSnapshot>("commit_query_transaction", {
    payload: { sessionId: sessionIdForTab(tabId) },
  });

export const rollbackQueryTransaction = (tabId: string) =>
  tauriInvoke<QueryTransactionSnapshot>("rollback_query_transaction", {
    payload: { sessionId: sessionIdForTab(tabId) },
  });

export const refreshQueryTransaction = (tabId: string) =>
  tauriInvoke<QueryTransactionSnapshot>("refresh_query_transaction_state", {
    payload: { sessionId: sessionIdForTab(tabId) },
  });

export async function closeQuerySessionForTab(tabId: string) {
  const binding = bindings.get(tabId);
  if (!binding) return;
  bindings.delete(tabId);
  binding.pendingRun?.reject({ kind: "sessionNotFound" });
  await tauriInvoke("close_query_session", {
    payload: { sessionId: binding.sessionId },
  });
}
async function closeQuerySession(sessionId: string) {
  await tauriInvoke("close_query_session", { payload: { sessionId } }).catch(
    () => undefined,
  );
}

export function hasQuerySessionBinding(tabId: string): boolean {
  return bindings.has(tabId);
}

export function resetQuerySessionChannelForTests(): void {
  bindings.clear();
  registration = null;
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }
  if (onFocusHeartbeat) {
    window.removeEventListener("focus", onFocusHeartbeat);
    onFocusHeartbeat = null;
  }
  if (onVisibilityHeartbeat) {
    document.removeEventListener("visibilitychange", onVisibilityHeartbeat);
    onVisibilityHeartbeat = null;
  }
}
