/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters anti-slop/require-safety-comment-for-type-assertion anti-slop/no-runtime-typeof -- Client tests mock the Tauri invoke boundary. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(() => Promise.resolve()),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import type {
  BrowseTableDataPayload,
  BrowseTableResult,
} from "@/lib/table-browse";
import {
  browseTable,
  closeTableBrowseForTab,
  resetTableBrowseClientForTab,
} from "@/lib/table-browse-client";
import { isTauri, tauriInvoke } from "@/lib/tauri";

const mockedIsTauri = vi.mocked(isTauri);
const mockedInvoke = vi.mocked(tauriInvoke);

const SECRET_SQL = "SELECT secret_sql FROM users WHERE email = $1";
const SECRET_FILTER = "secret_filter_value";
const SECRET_PARAM = "secret_param_value";
const SECRET_ROW = "secret_row_value";

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
    .map((value) => {
      if (typeof value === "string") return value;
      try {
        return JSON.stringify(value);
      } catch {
        return String(value);
      }
    })
    .join("\n");

const assertSecretsNeverLogged = () => {
  const text = loggedText();
  expect(text).not.toContain(SECRET_SQL);
  expect(text).not.toContain(SECRET_FILTER);
  expect(text).not.toContain(SECRET_PARAM);
  expect(text).not.toContain(SECRET_ROW);
};

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

const makeBrowseResult = (
  overrides: Partial<BrowseTableResult> = {},
): BrowseTableResult => ({
  requestId: 1,
  columns: [{ name: "email", castType: "text", nullable: true }],
  rows: [[SECRET_ROW]],
  identity: { kind: "primaryKey", columns: ["email"] },
  rowIdentity: [[SECRET_ROW]],
  pageInfo: { mode: "keyset", page: 1, hasMore: false, nextCursor: null },
  count: { kind: "estimated", value: 1 },
  inspection: {
    sql: SECRET_SQL,
    params: [{ kind: "text", value: SECRET_PARAM }],
  },
  omittedRows: 0,
  truncatedCells: 0,
  runtimeMs: 4,
  ...overrides,
});

const browsePayload = (
  tabId: string,
): Omit<BrowseTableDataPayload, "requestId"> => ({
  connectionId: "conn-1",
  tabId,
  schema: "public",
  table: "users",
  filters: [
    {
      kind: "comparison",
      column: "email",
      operator: "eq",
      value: SECRET_FILTER,
    },
  ],
  sort: [],
  pageRequest: { kind: "keyset", cursor: null },
  pageSize: 100,
  countPolicy: "estimated",
  refreshStructure: false,
});

beforeEach(() => {
  mockedIsTauri.mockReturnValue(true);
  mockedInvoke.mockReset();
  mockedInvoke.mockImplementation((_command, args) => {
    const requestId = requestIdFromArgs(args) ?? 0;
    return Promise.resolve(makeBrowseResult({ requestId }));
  });
  for (const spy of Object.values(consoleSpies)) {
    spy.mockClear();
    spy.mockImplementation(() => undefined);
  }
  resetTableBrowseClientForTab("tab-1");
  resetTableBrowseClientForTab("tab-2");
});

afterEach(() => {
  assertSecretsNeverLogged();
  resetTableBrowseClientForTab("tab-1");
  resetTableBrowseClientForTab("tab-2");
});

describe("table browse client request ids", () => {
  it("increments request ids per tab", async () => {
    await browseTable(browsePayload("tab-1"));
    await browseTable(browsePayload("tab-1"));
    await browseTable(browsePayload("tab-2"));
    await browseTable(browsePayload("tab-1"));

    const browseIds = mockedInvoke.mock.calls
      .filter((call) => call[0] === "browse_table_data")
      .map((call) => requestIdFromArgs(call[1]));
    expect(browseIds).toEqual([1, 2, 1, 3]);
  });

  it("treats a late success with an older requestId as superseded", async () => {
    let resolveFirst: (value: BrowseTableResult) => void = () => undefined;
    let resolveSecond: (value: BrowseTableResult) => void = () => undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveSecond = resolve;
          }),
      );

    const first = browseTable(browsePayload("tab-1"));
    const second = browseTable(browsePayload("tab-1"));

    resolveFirst(makeBrowseResult({ requestId: 1, rows: [["old"]] }));
    await expect(first).resolves.toEqual({ kind: "superseded" });

    const latest = makeBrowseResult({ requestId: 2, rows: [["new"]] });
    resolveSecond(latest);
    await expect(second).resolves.toEqual({ kind: "ok", value: latest });
  });

  it('maps invoke throwing {kind:"superseded"} to silent superseded', async () => {
    mockedInvoke.mockRejectedValueOnce({ kind: "superseded" });
    await expect(browseTable(browsePayload("tab-1"))).resolves.toEqual({
      kind: "superseded",
    });
  });

  it('maps invoke throwing {kind:"cancelled"} to cancelled', async () => {
    mockedInvoke.mockRejectedValueOnce({ kind: "cancelled" });
    await expect(browseTable(browsePayload("tab-1"))).resolves.toEqual({
      kind: "cancelled",
    });
  });

  it("maps other invoke errors to error results", async () => {
    mockedInvoke.mockRejectedValueOnce({
      kind: "timeout",
      operation: "browse",
    });
    await expect(browseTable(browsePayload("tab-1"))).resolves.toEqual({
      kind: "error",
      error: { kind: "timeout", operation: "browse" },
    });

    mockedInvoke.mockRejectedValueOnce({
      kind: "database",
      code: "57014",
      message: "canceling statement due to statement timeout",
      severity: "ERROR",
      position: null,
    });
    await expect(browseTable(browsePayload("tab-1"))).resolves.toEqual({
      kind: "error",
      error: {
        kind: "database",
        code: "57014",
        message: "canceling statement due to statement timeout",
        severity: "ERROR",
        position: null,
      },
    });
  });

  it("resets request ids when closeTableBrowseForTab runs", async () => {
    await browseTable(browsePayload("tab-1"));
    await browseTable(browsePayload("tab-1"));
    await closeTableBrowseForTab("conn-1", "tab-1");
    await browseTable(browsePayload("tab-1"));

    const browseIds = mockedInvoke.mock.calls
      .filter((call) => call[0] === "browse_table_data")
      .map((call) => requestIdFromArgs(call[1]));
    expect(browseIds).toEqual([1, 2, 1]);
    expect(mockedInvoke).toHaveBeenCalledWith("close_table_browse_for_tab", {
      payload: { connectionId: "conn-1", tabId: "tab-1" },
    });
  });
});
