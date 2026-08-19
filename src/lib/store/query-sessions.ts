/**
 * Persistent Query Sessions slice — owns PostgreSQL editor-session state,
 * event reduction, retained-result budgeting, and transaction commands.
 *
 * Relational Queries decides whether a query should use a persistent session;
 * this slice owns everything after that decision.
 */

import type { StateCreator } from "zustand";

import {
  applyEventBudget,
  defaultTransaction,
  releaseExecution,
} from "@/lib/query-session-budget";
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
} from "@/lib/query-session-channel";

import type {
  AppStoreState,
  QuerySessionState,
  QueryTransactionIsolation,
  QueryTransactionMode,
  QueryTransactionSnapshot,
} from "./types";

export type PersistentQueryResult =
  | { kind: "completed"; runtimeMs: number; rowCount: number }
  | { kind: "cancelled" };

export type QueryTransactionCommand =
  | { kind: "commit" }
  | { kind: "rollback" }
  | { kind: "refresh" }
  | { kind: "setMode"; mode: QueryTransactionMode }
  | { kind: "setIsolation"; isolation: QueryTransactionIsolation };

export type QuerySessionsSlice = {
  querySessions: Record<string, QuerySessionState>;
  executePersistentQuery: (
    tabId: string,
    connectionId: string,
    sql: string,
  ) => Promise<PersistentQueryResult>;
  cancelQuery: (tabId: string) => Promise<void>;
  applyQueryTransactionCommand: (
    tabId: string,
    command: QueryTransactionCommand,
  ) => Promise<void>;
  releaseQueryResults: (tabId: string) => void;
  markQuerySessionViewed: (tabId: string) => void;
  closeQuerySessionForTab: (tabId: string) => Promise<void>;
  closeQuerySessionsForConnection: (connectionId: string) => Promise<void>;
};

type SliceSet = Parameters<
  StateCreator<AppStoreState, [], [], QuerySessionsSlice>
>[0];

const patchSession = (
  set: SliceSet,
  tabId: string,
  patch: Partial<QuerySessionState>,
) =>
  set((state) => {
    const session = state.querySessions[tabId];
    return session
      ? {
          querySessions: {
            ...state.querySessions,
            [tabId]: { ...session, ...patch },
          },
        }
      : {};
  });

const invokeTransactionCommand = (
  tabId: string,
  command: QueryTransactionCommand,
): Promise<QueryTransactionSnapshot> => {
  switch (command.kind) {
    case "commit":
      return commitQueryTransaction(tabId);
    case "rollback":
      return rollbackQueryTransaction(tabId);
    case "refresh":
      return refreshQueryTransaction(tabId);
    case "setMode":
      return setQuerySessionTransactionMode(tabId, command.mode);
    case "setIsolation":
      return setQuerySessionTransactionIsolation(tabId, command.isolation);
  }
};

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
        transaction: defaultTransaction,
        execution: null,
        lastViewedAt: Date.now(),
        budgetOwners: [],
        state: "opening",
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
      patchSession(set, tabId, { transaction, state: "open" });
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
    return {
      kind: "completed",
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
    get().markQueryCancelling(tabId);
    await cancelQueryExecution(tabId, execution.id);
  },

  applyQueryTransactionCommand: async (tabId, command) => {
    const transaction = await invokeTransactionCommand(tabId, command);
    patchSession(set, tabId, { transaction });
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
    patchSession(set, tabId, { lastViewedAt: Date.now() }),

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
