/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/no-runtime-typeof -- Client tests mock the Tauri invoke boundary. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
}));

import type {
  AnalyzeResultSetResult,
  MutationPlan,
} from "@/lib/result-mutation";
import {
  analyzeResultSet,
  applyResultMutations,
  cancelResultMutation,
  clearVirtualKey,
  closeResultMutationForConnection,
  loadVirtualKey,
  previewResultMutations,
  resetResultMutationClientForTab,
  retryOnceAfterAnalysisExpired,
  saveVirtualKey,
} from "@/lib/result-mutation-client";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);

const SECRET_SQL = "SELECT secret_value FROM users";
const SECRET_VALUE = "private-row-value";

const consoleSpies = {
  log: vi.spyOn(console, "log"),
  debug: vi.spyOn(console, "debug"),
  info: vi.spyOn(console, "info"),
  warn: vi.spyOn(console, "warn"),
  error: vi.spyOn(console, "error"),
};

const loggedText = (): string =>
  Object.values(consoleSpies)
    .flatMap((spy) => spy.mock.calls)
    .flat()
    .map((value) => (typeof value === "string" ? value : JSON.stringify(value)))
    .join("\n");

const requestIdFromArgs = (args: unknown): number | undefined => {
  if (args === null || typeof args !== "object" || !("payload" in args)) {
    return undefined;
  }
  const payload = args.payload;
  if (payload === null || typeof payload !== "object") return undefined;
  if (!("requestId" in payload) || typeof payload.requestId !== "number") {
    return undefined;
  }
  return payload.requestId;
};

const analysisResult = (requestId: number): AnalyzeResultSetResult => ({
  requestId,
  analysisId: requestId + 10,
  columns: [],
  tables: [],
  statement: { kind: "analyzed" },
});

const analysisPayload = (tabId: string, connectionId = "conn-1") => ({
  connectionId,
  tabId,
  source: { kind: "statement" as const, sql: SECRET_SQL },
  refreshStructure: false,
});

const plan: MutationPlan = {
  operations: [
    {
      kind: "insert",
      table: { schema: "public", table: "users" },
      values: [{ column: "name", value: SECRET_VALUE }],
    },
  ],
};

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  mockedInvoke.mockResolvedValue(undefined);
  for (const spy of Object.values(consoleSpies)) {
    spy.mockClear();
    spy.mockImplementation(() => undefined);
  }
  resetResultMutationClientForTab("tab-1");
  resetResultMutationClientForTab("tab-2");
});

afterEach(() => {
  const logged = loggedText();
  expect(logged).not.toContain(SECRET_SQL);
  expect(logged).not.toContain(SECRET_VALUE);
  resetResultMutationClientForTab("tab-1");
  resetResultMutationClientForTab("tab-2");
});

describe("result mutation client", () => {
  it("issues monotonic analysis request ids independently per tab", async () => {
    mockedInvoke.mockImplementation((_command, args) =>
      Promise.resolve(analysisResult(requestIdFromArgs(args) ?? 0)),
    );

    await analyzeResultSet(analysisPayload("tab-1"));
    await analyzeResultSet(analysisPayload("tab-1"));
    await analyzeResultSet(analysisPayload("tab-2"));

    expect(
      mockedInvoke.mock.calls.map((call) => requestIdFromArgs(call[1])),
    ).toEqual([1, 2, 1]);
  });

  it("silently supersedes a late analysis success and failure", async () => {
    let resolveFirst: (value: AnalyzeResultSetResult) => void = () => undefined;
    let rejectSecond: (reason: unknown) => void = () => undefined;
    let resolveThird: (value: AnalyzeResultSetResult) => void = () => undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            rejectSecond = reject;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveThird = resolve;
          }),
      );

    const first = analyzeResultSet(analysisPayload("tab-1"));
    const second = analyzeResultSet(analysisPayload("tab-1"));
    resolveFirst(analysisResult(1));
    await expect(first).resolves.toEqual({ kind: "superseded" });

    const third = analyzeResultSet(analysisPayload("tab-1"));
    rejectSecond({ kind: "database", message: SECRET_VALUE });
    await expect(second).resolves.toEqual({ kind: "superseded" });
    resolveThird(analysisResult(3));
    await expect(third).resolves.toEqual({
      kind: "ok",
      value: analysisResult(3),
    });
  });

  it("supersedes an in-flight analysis when a tab changes connection", async () => {
    let resolveOld: (value: AnalyzeResultSetResult) => void = () => undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveOld = resolve;
          }),
      )
      .mockResolvedValueOnce(analysisResult(1));

    const old = analyzeResultSet(analysisPayload("tab-1", "conn-1"));
    await expect(
      analyzeResultSet(analysisPayload("tab-1", "conn-2")),
    ).resolves.toEqual({ kind: "ok", value: analysisResult(1) });
    resolveOld(analysisResult(1));

    await expect(old).resolves.toEqual({ kind: "superseded" });
  });

  it("does not let apply request ids supersede analysis", async () => {
    let resolveAnalysis: (value: AnalyzeResultSetResult) => void = () =>
      undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveAnalysis = resolve;
          }),
      )
      .mockResolvedValueOnce({ operations: [], runtimeMs: 1 });

    const analysis = analyzeResultSet(analysisPayload("tab-1"));
    await applyResultMutations({
      connectionId: "conn-1",
      tabId: "tab-1",
      analysisId: 11,
      plan,
    });
    resolveAnalysis(analysisResult(1));

    await expect(analysis).resolves.toEqual({
      kind: "ok",
      value: analysisResult(1),
    });
    expect(requestIdFromArgs(mockedInvoke.mock.calls[1]?.[1])).toBe(1);
  });

  it("invokes every command with the exact payload envelope", async () => {
    mockedInvoke
      .mockResolvedValueOnce({ statements: [] })
      .mockResolvedValueOnce({ operations: [], runtimeMs: 2 })
      .mockResolvedValueOnce({ cancelRequested: true })
      .mockResolvedValueOnce({ version: 1, columns: ["email"] })
      .mockResolvedValue(undefined);

    await previewResultMutations({
      connectionId: "conn-1",
      tabId: "tab-1",
      analysisId: 7,
      plan,
    });
    await applyResultMutations({
      connectionId: "conn-1",
      tabId: "tab-1",
      analysisId: 7,
      plan,
    });
    await cancelResultMutation({ connectionId: "conn-1", tabId: "tab-1" });
    await loadVirtualKey({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
    });
    await saveVirtualKey({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
      columns: ["email"],
    });
    await clearVirtualKey({
      connectionId: "conn-1",
      schema: "public",
      table: "users",
    });

    expect(mockedInvoke.mock.calls.map((call) => call[0])).toEqual([
      "preview_result_mutations",
      "apply_result_mutations",
      "cancel_result_mutation",
      "load_virtual_key",
      "save_virtual_key",
      "clear_virtual_key",
    ]);
    expect(mockedInvoke.mock.calls[1]?.[1]).toEqual({
      payload: {
        connectionId: "conn-1",
        tabId: "tab-1",
        analysisId: 7,
        plan,
        requestId: 1,
      },
    });
  });

  it("maps cancellation, supersession, typed errors, and malformed errors", async () => {
    mockedInvoke
      .mockRejectedValueOnce({ kind: "cancelled" })
      .mockRejectedValueOnce({ kind: "superseded" })
      .mockRejectedValueOnce({ kind: "busy" })
      .mockRejectedValueOnce({ kind: "conflict", opIndex: 0 })
      .mockRejectedValueOnce({ kind: "conflict", opIndex: "bad" });

    const payload = {
      connectionId: "conn-1",
      tabId: "tab-1",
      analysisId: 7,
      plan,
    };
    await expect(previewResultMutations(payload)).resolves.toEqual({
      kind: "cancelled",
    });
    await expect(previewResultMutations(payload)).resolves.toEqual({
      kind: "superseded",
    });
    await expect(previewResultMutations(payload)).resolves.toEqual({
      kind: "error",
      error: { kind: "busy" },
    });
    await expect(previewResultMutations(payload)).resolves.toEqual({
      kind: "error",
      error: { kind: "conflict", opIndex: 0 },
    });
    await expect(previewResultMutations(payload)).resolves.toEqual({
      kind: "error",
      error: { kind: "connectionLost" },
    });
  });

  it("retries analysisExpired only once after successful recovery", async () => {
    const operation = vi
      .fn()
      .mockResolvedValueOnce({
        kind: "error",
        error: { kind: "analysisExpired" },
      })
      .mockResolvedValueOnce({ kind: "ok", value: "preview" });
    const recover = vi.fn().mockResolvedValue({
      kind: "ok",
      value: analysisResult(2),
    });

    await expect(
      retryOnceAfterAnalysisExpired(operation, recover),
    ).resolves.toEqual({ kind: "ok", value: "preview" });
    expect(operation).toHaveBeenCalledTimes(2);
    expect(recover).toHaveBeenCalledTimes(1);

    operation.mockReset().mockResolvedValue({
      kind: "error",
      error: { kind: "analysisExpired" },
    });
    await expect(
      retryOnceAfterAnalysisExpired(operation, recover),
    ).resolves.toEqual({
      kind: "error",
      error: { kind: "analysisExpired" },
    });
    expect(operation).toHaveBeenCalledTimes(2);
    expect(recover).toHaveBeenCalledTimes(2);
  });

  it("stops recovery when re-analysis does not succeed", async () => {
    const operation = vi.fn().mockResolvedValue({
      kind: "error",
      error: { kind: "analysisExpired" },
    });
    const recover = vi.fn().mockResolvedValue({
      kind: "error",
      error: { kind: "notAnalyzable", reason: { kind: "noTableOrigins" } },
    });

    await expect(
      retryOnceAfterAnalysisExpired(operation, recover),
    ).resolves.toEqual({
      kind: "error",
      error: { kind: "notAnalyzable", reason: { kind: "noTableOrigins" } },
    });
    expect(operation).toHaveBeenCalledTimes(1);
  });

  it("resets matching tab request state when a connection closes", async () => {
    mockedInvoke.mockImplementation((command, args) =>
      command === "analyze_result_set"
        ? Promise.resolve(analysisResult(requestIdFromArgs(args) ?? 0))
        : Promise.resolve(undefined),
    );

    await analyzeResultSet(analysisPayload("tab-1", "conn-1"));
    await analyzeResultSet(analysisPayload("tab-2", "conn-2"));
    await closeResultMutationForConnection("conn-1");
    await analyzeResultSet(analysisPayload("tab-1", "conn-1"));
    await analyzeResultSet(analysisPayload("tab-2", "conn-2"));

    const analysisIds = mockedInvoke.mock.calls
      .filter((call) => call[0] === "analyze_result_set")
      .map((call) => requestIdFromArgs(call[1]));
    expect(analysisIds).toEqual([1, 1, 1, 2]);
    expect(mockedInvoke).toHaveBeenCalledWith(
      "close_result_mutation_for_connection",
      { payload: { connectionId: "conn-1" } },
    );
  });

  it("fails closed without invoking commands outside Tauri", async () => {
    mockedIsTauri.mockReturnValue(false);
    await expect(analyzeResultSet(analysisPayload("tab-1"))).resolves.toEqual({
      kind: "error",
      error: { kind: "connectionLost" },
    });
    await expect(
      previewResultMutations({
        connectionId: "conn-1",
        tabId: "tab-1",
        analysisId: 7,
        plan,
      }),
    ).resolves.toEqual({
      kind: "error",
      error: { kind: "connectionLost" },
    });
    expect(mockedInvoke).not.toHaveBeenCalled();
  });
});
