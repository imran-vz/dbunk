/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion -- Store tests mock the query-session channel boundary. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

vi.mock("@/lib/query-session-channel", () => ({
  cancelQueryExecution: vi.fn(() => Promise.resolve()),
  closeQuerySessionForTab: vi.fn(() => Promise.resolve()),
  commitQueryTransaction: vi.fn(() => Promise.resolve()),
  executeQuerySession: vi.fn(() => Promise.resolve()),
  openQuerySession: vi.fn(() => Promise.resolve()),
  querySessionChannelsAvailable: vi.fn(() => false),
  refreshQueryTransaction: vi.fn(() => Promise.resolve()),
  rollbackQueryTransaction: vi.fn(() => Promise.resolve()),
  setQuerySessionTransactionIsolation: vi.fn(() => Promise.resolve()),
  setQuerySessionTransactionMode: vi.fn(() => Promise.resolve()),
}));

import {
  applyEventBudget,
  defaultTransaction,
  encodedValueBytes,
  evictInactiveResults,
  newExecution,
  omitEnvelopePayload,
  reduceSessionEvent,
  releaseExecution,
  retainedBytes,
} from "@/lib/query-session-budget";
import type { QueryEventEnvelope } from "@/lib/query-session-channel";
import {
  commitQueryTransaction,
  executeQuerySession,
  openQuerySession,
  querySessionChannelsAvailable,
  refreshQueryTransaction,
  rollbackQueryTransaction,
  setQuerySessionTransactionIsolation,
  setQuerySessionTransactionMode,
} from "@/lib/query-session-channel";
import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";
import type { QueryExecution, QuerySessionState } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);
const mockedChannelsAvailable = vi.mocked(querySessionChannelsAvailable);
const mockedOpenQuerySession = vi.mocked(openQuerySession);
const mockedExecuteQuerySession = vi.mocked(executeQuerySession);
const mockedCommitQueryTransaction = vi.mocked(commitQueryTransaction);
const mockedRefreshQueryTransaction = vi.mocked(refreshQueryTransaction);
const mockedRollbackQueryTransaction = vi.mocked(rollbackQueryTransaction);
const mockedSetQuerySessionTransactionMode = vi.mocked(
  setQuerySessionTransactionMode,
);
const mockedSetQuerySessionTransactionIsolation = vi.mocked(
  setQuerySessionTransactionIsolation,
);

const initialStoreState = useAppStore.getState();

const resetStore = () => {
  useAppStore.setState(initialStoreState, true);
  useAppStore.setState({ activeConnectionId: "conn-1" });
};

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedChannelsAvailable.mockReturnValue(false);
  resetStore();
});

afterEach(() => {
  resetStore();
});

const makeSession = (
  tabId: string,
  overrides: Partial<QuerySessionState> = {},
): QuerySessionState => ({
  id: `session-${tabId}`,
  tabId,
  connectionId: "conn-1",
  generation: 1,
  transaction: defaultTransaction,
  execution: null,
  lastViewedAt: 1,
  budgetOwners: [],
  state: "open",
  ...overrides,
});

const envelope = (
  overrides: Partial<QueryEventEnvelope> & {
    event: QueryEventEnvelope["event"];
  },
): QueryEventEnvelope => ({
  sessionId: "session-1",
  tabId: "tab-1",
  connectionId: "conn-1",
  generation: 1,
  sequence: 1,
  executionId: "exec-1",
  requiresAck: false,
  ...overrides,
});

const completedExecution = (
  overrides: Partial<QueryExecution> = {},
): QueryExecution => ({
  ...newExecution("exec-1"),
  status: "completed",
  completedAt: new Date().toISOString(),
  runtimeMs: 12,
  ...overrides,
});

describe("query-session budget machinery", () => {
  it("evicts inactive results in LRU order and skips the active tab and running executions", () => {
    const sessions = {
      "tab-old": makeSession("tab-old", {
        lastViewedAt: 1,
        execution: completedExecution({ retainedBytes: 40 }),
      }),
      "tab-running": makeSession("tab-running", {
        lastViewedAt: 0,
        execution: { ...newExecution("exec-run"), retainedBytes: 40 },
      }),
      "tab-mid": makeSession("tab-mid", {
        lastViewedAt: 2,
        execution: completedExecution({ retainedBytes: 40 }),
      }),
      "tab-active": makeSession("tab-active", {
        lastViewedAt: 0,
        execution: completedExecution({ retainedBytes: 40 }),
      }),
    };

    const retained = evictInactiveResults(sessions, "tab-active", 130);

    expect(sessions["tab-old"]?.execution?.tombstone).not.toBeNull();
    expect(sessions["tab-old"]?.execution?.retainedBytes).toBe(0);
    expect(sessions["tab-mid"]?.execution?.tombstone).toBeNull();
    expect(sessions["tab-running"]?.execution?.tombstone).toBeNull();
    expect(sessions["tab-running"]?.execution?.status).toBe("running");
    expect(sessions["tab-active"]?.execution?.tombstone).toBeNull();
    expect(retained).toBe(120);
  });

  it("writes a tombstone and zeros retainedBytes on release", () => {
    const execution = completedExecution({
      retainedBytes: 64,
      resultSets: [
        {
          index: 0,
          columns: ["id"],
          rowChunks: [[["1"]]],
          rowCount: 1,
          partial: false,
          completed: true,
        },
      ],
      notices: [{ severity: "NOTICE", message: "ok" }],
    });
    const released = releaseExecution(execution);
    expect(released.retainedBytes).toBe(0);
    expect(released.resultSets).toEqual([]);
    expect(released.tombstone).toEqual({
      status: "completed",
      resultCount: 1,
      rowCount: 1,
      noticeCount: 1,
      omittedCount: 0,
      runtimeMs: 12,
      releasedBytes: 64,
      completedAt: execution.completedAt,
      reason: "globalBudget",
    });
  });

  it("keeps incremental retainedBytes consistent: add then release returns to zero", () => {
    const columns = ["id", "name"];
    const rows = [
      ["1", "ada"],
      ["2", "grace"],
    ];
    let session = makeSession("tab-1");
    session = reduceSessionEvent(
      session,
      envelope({ event: { kind: "executionStarted" } }),
    );
    session = reduceSessionEvent(
      session,
      envelope({
        sequence: 2,
        event: { kind: "resultSetStarted", resultSetIndex: 0, columns },
      }),
    );
    session = reduceSessionEvent(
      session,
      envelope({
        sequence: 3,
        event: { kind: "rowBatch", resultSetIndex: 0, rows },
      }),
    );

    const expected = encodedValueBytes(columns) + encodedValueBytes(rows);
    expect(session.execution?.retainedBytes).toBe(expected);
    expect(session.execution?.resultSets[0]?.rowChunks).toEqual([rows]);

    const execution = session.execution;
    if (!execution) throw new Error("expected an execution");
    const released = releaseExecution(execution);
    expect(released.retainedBytes).toBe(0);
    expect(released.tombstone?.releasedBytes).toBe(expected);
    expect(
      retainedBytes({
        "tab-1": { ...session, execution: released },
      }),
    ).toBe(0);
  });

  it("omits over-budget row batches and records frontend truncation", () => {
    const rows = [["a very long cell that exceeds the tiny test budget"]];
    let session = makeSession("tab-1");
    session = reduceSessionEvent(
      session,
      envelope({ event: { kind: "executionStarted" } }),
    );
    session = reduceSessionEvent(
      session,
      envelope({
        sequence: 2,
        event: {
          kind: "resultSetStarted",
          resultSetIndex: 0,
          columns: ["id"],
        },
      }),
    );
    const omitted = omitEnvelopePayload(
      session,
      envelope({
        sequence: 3,
        event: { kind: "rowBatch", resultSetIndex: 0, rows },
      }),
    );
    expect(omitted.execution?.omittedRows).toBe(1);
    expect(omitted.execution?.truncationReasons).toContain(
      "frontendGlobalBudget",
    );

    const budgeted = applyEventBudget(
      {
        querySessions: { "tab-1": session },
        activeTabId: "tab-1",
        workspaceTabs: [{ id: "tab-1", label: "query_1.sql" }],
      },
      "tab-1",
      envelope({
        sequence: 3,
        event: { kind: "rowBatch", resultSetIndex: 0, rows },
      }),
      8,
    );
    expect(budgeted.retainMoreRows).toBe(false);
    expect(
      budgeted.querySessions["tab-1"]?.execution?.truncationReasons,
    ).toContain("frontendGlobalBudget");
    expect(
      budgeted.querySessions["tab-1"]?.execution?.resultSets[0]?.rowChunks,
    ).toEqual([[]]);
  });

  it("lists other budget owners when the current tab is over budget", () => {
    const rows = [["another oversized payload for the owner listing"]];
    const other = makeSession("tab-other", {
      lastViewedAt: 5,
      execution: completedExecution({ retainedBytes: 32 }),
    });
    let current = makeSession("tab-1");
    current = reduceSessionEvent(
      current,
      envelope({ event: { kind: "executionStarted" } }),
    );
    current = reduceSessionEvent(
      current,
      envelope({
        sequence: 2,
        event: {
          kind: "resultSetStarted",
          resultSetIndex: 0,
          columns: ["id"],
        },
      }),
    );
    const budgeted = applyEventBudget(
      {
        querySessions: { "tab-1": current, "tab-other": other },
        activeTabId: "tab-other",
        workspaceTabs: [
          { id: "tab-1", label: "query_1.sql" },
          { id: "tab-other", label: "heavy.sql" },
        ],
      },
      "tab-1",
      envelope({
        sequence: 3,
        event: { kind: "rowBatch", resultSetIndex: 0, rows },
      }),
      8,
    );
    expect(budgeted.querySessions["tab-1"]?.budgetOwners).toEqual([
      { tabId: "tab-other", label: "heavy.sql", retainedBytes: 32 },
    ]);
  });

  it("retains executionCompleted errors even when over budget", () => {
    const error = {
      code: "42P01",
      message: "missing",
      severity: "ERROR",
      position: 1,
    };
    const session = reduceSessionEvent(
      makeSession("tab-1"),
      envelope({ event: { kind: "executionStarted" } }),
    );
    const omitted = omitEnvelopePayload(
      session,
      envelope({
        sequence: 2,
        event: {
          kind: "executionCompleted",
          status: "failed",
          transaction: defaultTransaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error,
        },
      }),
    );
    expect(omitted.execution?.error).toEqual(error);
  });
});

describe("query execution replacement", () => {
  const seedStagedExecution = () => {
    const tabId = "tab-1";
    useAppStore.setState({
      workspaceTabs: [
        {
          id: tabId,
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select id from users",
        },
      ],
      querySessions: {
        [tabId]: makeSession(tabId, { execution: completedExecution() }),
      },
    });
    const handle = useAppStore.getState().openMutationDraft({
      owner: {
        kind: "query",
        tabId,
        executionId: "exec-1",
        resultSetIndex: 0,
      },
      connectionId: "conn-1",
      source: { kind: "statement", sql: "select id from users" },
    });
    if (!handle) throw new Error("Expected mutation draft handle");
    useAppStore.getState().stageMutationDraftInsert(handle.scope, {
      table: { schema: "public", table: "users" },
      values: [{ column: "id", value: "2" }],
    });
    return { handle, tabId };
  };

  it("fails closed when a non-UI caller would replace staged changes", async () => {
    mockedInvoke.mockClear();
    const { handle, tabId } = seedStagedExecution();

    await expect(useAppStore.getState().runQuery(tabId)).resolves.toEqual({
      kind: "noop",
    });

    expect(useAppStore.getState().mutationDrafts[handle.scope]).toBeDefined();
    expect(mockedInvoke).not.toHaveBeenCalled();
  });

  it("drops the exact execution draft through the named action after confirmation", async () => {
    mockedInvoke.mockReset();
    const { handle, tabId } = seedStagedExecution();
    const confirmDiscardStagedChanges = vi.fn(() => true);
    mockedInvoke
      .mockResolvedValueOnce({
        columns: ["id"],
        rows: [["2"]],
        runtimeMs: 1,
        rowCount: 1,
      })
      .mockResolvedValueOnce([]);

    await useAppStore.getState().runQuery(tabId, {
      confirmDiscardStagedChanges,
    });

    expect(confirmDiscardStagedChanges).toHaveBeenCalledWith(1);
    expect(useAppStore.getState().mutationDrafts[handle.scope]).toBeUndefined();
    expect(mockedInvoke).toHaveBeenCalledWith("run_query", {
      payload: {
        connectionId: "conn-1",
        query: "select id from users",
      },
    });
  });
});

describe("typed query-session commands", () => {
  const session = makeSession("tab-1");

  it("dispatches transaction commands through one store action", async () => {
    const manual = { ...session.transaction, mode: "manual" as const };
    const isolated = {
      ...manual,
      manualIsolation: "serializable" as const,
    };
    mockedSetQuerySessionTransactionMode.mockResolvedValueOnce(manual);
    mockedSetQuerySessionTransactionIsolation.mockResolvedValueOnce(isolated);
    mockedCommitQueryTransaction.mockResolvedValueOnce(manual);
    mockedRollbackQueryTransaction.mockResolvedValueOnce(manual);
    mockedRefreshQueryTransaction.mockResolvedValueOnce(manual);
    useAppStore.setState({ querySessions: { "tab-1": session } });

    await useAppStore.getState().applyQueryTransactionCommand("tab-1", {
      kind: "setMode",
      mode: "manual",
    });
    await useAppStore.getState().applyQueryTransactionCommand("tab-1", {
      kind: "setIsolation",
      isolation: "serializable",
    });
    await useAppStore
      .getState()
      .applyQueryTransactionCommand("tab-1", { kind: "commit" });
    await useAppStore
      .getState()
      .applyQueryTransactionCommand("tab-1", { kind: "rollback" });
    await useAppStore
      .getState()
      .applyQueryTransactionCommand("tab-1", { kind: "refresh" });

    expect(mockedSetQuerySessionTransactionMode).toHaveBeenCalledWith(
      "tab-1",
      "manual",
    );
    expect(mockedSetQuerySessionTransactionIsolation).toHaveBeenCalledWith(
      "tab-1",
      "serializable",
    );
    expect(mockedCommitQueryTransaction).toHaveBeenCalledWith("tab-1");
    expect(mockedRollbackQueryTransaction).toHaveBeenCalledWith("tab-1");
    expect(mockedRefreshQueryTransaction).toHaveBeenCalledWith("tab-1");
    expect(useAppStore.getState().querySessions["tab-1"]?.transaction).toEqual(
      manual,
    );
  });
});

describe("persistent query session outcomes", () => {
  const seedQueryTab = (overrides?: { id?: string; query?: string }) => {
    const id = overrides?.id ?? "tab-1";
    const query = overrides?.query ?? "select 1";
    useAppStore.setState({
      workspaceTabs: [
        {
          id,
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query,
        },
      ],
      activeConnectionId: "conn-1",
      connections: [
        {
          id: "conn-1",
          name: "Local",
          database: "postgres",
          status: "Connected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
        },
      ],
    });
    return id;
  };

  it("retries a typed policy refusal only after shared confirmation", async () => {
    const tabId = seedQueryTab({ query: "delete from users" });
    useAppStore.setState({
      querySessions: { [tabId]: makeSession(tabId) },
    });
    mockedExecuteQuerySession.mockReset();
    mockedExecuteQuerySession
      .mockRejectedValueOnce({
        kind: "policyNeedsConfirmation",
        statements: [
          {
            index: 0,
            class: "dml",
            unbounded: true,
            destructive: true,
          },
        ],
      })
      .mockImplementationOnce(async (_runTabId, executionId) => {
        const current = useAppStore.getState().querySessions[tabId];
        if (!current) throw new Error("expected query session");
        useAppStore.setState({
          querySessions: {
            [tabId]: {
              ...current,
              execution: completedExecution({ id: executionId }),
            },
          },
        });
      });

    const outcome = useAppStore
      .getState()
      .executePersistentQuery(tabId, "conn-1", "delete from users");

    await vi.waitFor(() => expect(getSafetyConfirmation()).not.toBeNull());
    expect(mockedExecuteQuerySession.mock.calls[0]).toHaveLength(3);
    resolveSafetyConfirmation(true);

    await expect(outcome).resolves.toEqual({
      kind: "completed",
      runtimeMs: 12,
      rowCount: 0,
    });
    expect(mockedExecuteQuerySession).toHaveBeenNthCalledWith(
      2,
      tabId,
      expect.any(String),
      "delete from users",
      true,
    );
  });

  it("retains a non-dismissable policy-blocked reason on the session", async () => {
    const tabId = seedQueryTab({ query: "update users set active = false" });
    useAppStore.setState({
      querySessions: { [tabId]: makeSession(tabId) },
    });
    mockedExecuteQuerySession.mockReset();
    mockedExecuteQuerySession.mockRejectedValueOnce({
      kind: "policyBlocked",
      reason: "This connection is read-only.",
    });

    await expect(
      useAppStore
        .getState()
        .executePersistentQuery(
          tabId,
          "conn-1",
          "update users set active = false",
        ),
    ).rejects.toThrow("Edit the connection to unlock writes.");
    expect(useAppStore.getState().querySessions[tabId]?.policyRefusal).toBe(
      "This connection is read-only. Edit the connection to unlock writes.",
    );
  });

  it("does not open confirmation for a malformed policy refusal", async () => {
    const tabId = seedQueryTab({ query: "delete from users" });
    useAppStore.setState({
      querySessions: { [tabId]: makeSession(tabId) },
    });
    const malformedRefusal = {
      kind: "policyNeedsConfirmation",
      statements: [{ destructive: true }],
    };
    mockedExecuteQuerySession.mockReset();
    mockedExecuteQuerySession.mockRejectedValueOnce(malformedRefusal);

    await expect(
      useAppStore
        .getState()
        .executePersistentQuery(tabId, "conn-1", "delete from users"),
    ).rejects.toEqual(malformedRefusal);
    expect(getSafetyConfirmation()).toBeNull();
    expect(mockedExecuteQuerySession).toHaveBeenCalledOnce();
  });

  it("does not record a cancelled execution as success", async () => {
    const tabId = seedQueryTab();
    useAppStore.setState({
      queryPreviews: {
        [tabId]: {
          columns: ["previous"],
          rows: [["result"]],
          runtime: "1 ms",
          rowCount: "1",
          cache: "Cold",
        },
      },
    });
    mockedChannelsAvailable.mockReturnValue(true);
    let handler: Parameters<typeof openQuerySession>[0]["handler"] | undefined;
    mockedOpenQuerySession.mockImplementationOnce(async (input) => {
      handler = input.handler;
      return defaultTransaction;
    });
    mockedExecuteQuerySession.mockImplementationOnce(async (_tabId, id) => {
      const send = handler;
      if (!send) throw new Error("session handler was not registered");
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 1,
        executionId: id,
        requiresAck: false,
        event: { kind: "executionStarted" },
      });
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 2,
        executionId: id,
        requiresAck: true,
        event: {
          kind: "executionCompleted",
          status: "cancelled",
          transaction: defaultTransaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error: null,
        },
      });
    });

    const outcome = await useAppStore.getState().runQuery(tabId);

    expect(outcome).toEqual({ kind: "cancelled" });
    expect(useAppStore.getState().queryHistory).toEqual([]);
    expect(useAppStore.getState().queryPreviews[tabId]?.columns).toEqual([
      "previous",
    ]);
    expect(useAppStore.getState().queryStatus[tabId]).toBeUndefined();
  });

  it("does not store session rows in queryPreviews", async () => {
    const tabId = seedQueryTab();
    useAppStore.setState({
      queryPreviews: {
        [tabId]: {
          columns: ["stale"],
          rows: [["old"]],
          runtime: "1 ms",
          rowCount: "1",
          cache: "Cold",
        },
      },
    });
    mockedChannelsAvailable.mockReturnValue(true);
    let handler: Parameters<typeof openQuerySession>[0]["handler"] | undefined;
    mockedOpenQuerySession.mockImplementationOnce(async (input) => {
      handler = input.handler;
      return defaultTransaction;
    });
    mockedExecuteQuerySession.mockImplementationOnce(async (_tabId, id) => {
      const send = handler;
      if (!send) throw new Error("session handler was not registered");
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 1,
        executionId: id,
        requiresAck: false,
        event: { kind: "executionStarted" },
      });
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 2,
        executionId: id,
        requiresAck: false,
        event: {
          kind: "resultSetStarted",
          resultSetIndex: 0,
          columns: ["id"],
        },
      });
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 3,
        executionId: id,
        requiresAck: false,
        event: {
          kind: "rowBatch",
          resultSetIndex: 0,
          rows: [["1"]],
        },
      });
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 4,
        executionId: id,
        requiresAck: false,
        event: {
          kind: "resultSetCompleted",
          resultSetIndex: 0,
          rowCount: 1,
          partial: false,
        },
      });
      send({
        sessionId: "session-1",
        tabId,
        connectionId: "conn-1",
        generation: 1,
        sequence: 5,
        executionId: id,
        requiresAck: true,
        event: {
          kind: "executionCompleted",
          status: "completed",
          transaction: defaultTransaction,
          omittedRows: 0,
          omittedResultSets: 0,
          omittedNotices: 0,
          omittedMetadataBytes: 0,
          truncationReasons: [],
          error: null,
        },
      });
    });

    const outcome = await useAppStore.getState().runQuery(tabId);

    expect(outcome).toEqual({
      kind: "completed",
      runtimeMs: expect.any(Number),
      rowCount: 1,
      preview: null,
    });
    expect(useAppStore.getState().queryPreviews[tabId]).toBeUndefined();
    expect(
      useAppStore.getState().querySessions[tabId]?.execution?.resultSets[0]
        ?.rowChunks,
    ).toEqual([[["1"]]]);
  });

  it("does not start another run while cancellation is pending", async () => {
    const tabId = seedQueryTab();
    useAppStore.setState({
      queryStatus: { [tabId]: { state: "cancelling" } },
    });

    await expect(useAppStore.getState().runQuery(tabId)).resolves.toEqual({
      kind: "noop",
    });
  });

  it("closeTab does not prompt from the store", async () => {
    const confirmSpy = vi.spyOn(window, "confirm");
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "query",
          label: "query_1.sql",
          connectionId: "conn-1",
          schema: "public",
          query: "select 1",
        },
      ],
      querySessions: {
        "tab-1": makeSession("tab-1", {
          transaction: { ...defaultTransaction, status: "active" },
        }),
      },
    });

    await useAppStore.getState().closeTab("tab-1");

    expect(confirmSpy).not.toHaveBeenCalled();
    expect(useAppStore.getState().workspaceTabs).toEqual([]);
    confirmSpy.mockRestore();
  });
});

describe("production query retargeting", () => {
  it("fails closed unless the caller confirms the production target", async () => {
    useAppStore.setState({
      workspaceTabs: [
        {
          id: "tab-1",
          kind: "query",
          label: "query.sql",
          connectionId: "conn-dev",
          schema: "public",
          query: "select 1",
        },
      ],
      connections: [
        {
          id: "conn-dev",
          name: "Dev",
          database: "app",
          status: "Connected",
          engine: "PostgreSQL",
          host: "localhost",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
          environment: "development",
        },
        {
          id: "conn-prod",
          name: "Primary",
          database: "app",
          status: "Connected",
          engine: "PostgreSQL",
          host: "prod.internal",
          port: 5432,
          user: "postgres",
          password: "",
          role: "admin",
          latency: "--",
          ssl: true,
          environment: "production",
        },
      ],
    });

    await expect(
      useAppStore.getState().retargetQueryTab("tab-1", "conn-prod"),
    ).resolves.toBe(false);
    const confirmProductionTarget = vi.fn(() => true);
    await expect(
      useAppStore.getState().retargetQueryTab("tab-1", "conn-prod", {
        confirmProductionTarget,
      }),
    ).resolves.toBe(true);
    expect(confirmProductionTarget).toHaveBeenCalledWith(
      expect.objectContaining({ id: "conn-prod", environment: "production" }),
    );
  });
});
