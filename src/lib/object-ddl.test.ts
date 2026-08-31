/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- These tests control the typed Tauri rejection boundary. */
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => true),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import {
  applyObjectDdlWithSafetyConfirmation,
  formatObjectDdlError,
  loadPgDropImpact,
  objectDdlRefreshScope,
  previewObjectDdl,
} from "@/lib/object-ddl";
import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";
import type { Connection, DdlStatementSummary, PgObjectRef } from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);

const connection: Connection = {
  id: "conn-1",
  name: "Staging",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "8 ms",
  ssl: false,
  environment: "staging",
  safeMode: "protected",
  readOnly: false,
};

const viewRef: PgObjectRef = {
  kind: "view",
  schema: "public",
  name: "active_users",
  identityArgs: null,
};

beforeEach(() => {
  mockedInvoke.mockReset();
  resolveSafetyConfirmation(false);
});

describe("object DDL client", () => {
  it("invokes preview and impact commands through the typed payload boundary", async () => {
    mockedInvoke
      .mockResolvedValueOnce({ statements: [], groups: [] })
      .mockResolvedValueOnce({ dependents: [], truncated: false });

    await expect(
      previewObjectDdl({
        connectionId: connection.id,
        ops: [{ op: "dropObject", reference: viewRef, cascade: false }],
      }),
    ).resolves.toEqual({
      kind: "ok",
      value: { statements: [], groups: [] },
    });
    await expect(
      loadPgDropImpact({ connectionId: connection.id, reference: viewRef }),
    ).resolves.toEqual({
      kind: "ok",
      value: { dependents: [], truncated: false },
    });

    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "preview_object_ddl", {
      payload: {
        connectionId: connection.id,
        ops: [{ op: "dropObject", reference: viewRef, cascade: false }],
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "load_pg_drop_impact", {
      payload: { connectionId: connection.id, reference: viewRef },
    });
  });

  it("uses the typed DDL confirmation summaries and retries confirmed", async () => {
    const statements: DdlStatementSummary[] = [
      {
        index: 0,
        summary: "Drop view public.active_users",
        destructive: true,
        transactional: true,
      },
    ];
    mockedInvoke
      .mockRejectedValueOnce({
        kind: "policyNeedsConfirmation",
        statements,
      })
      .mockResolvedValueOnce({ appliedStatements: 1, runtimeMs: 6 });
    const ops = [
      { op: "dropObject", reference: viewRef, cascade: false },
    ] as const;

    const result = applyObjectDdlWithSafetyConfirmation(
      { connectionId: connection.id, ops: [...ops] },
      connection,
    );

    await vi.waitFor(() => expect(getSafetyConfirmation()).not.toBeNull());
    expect(getSafetyConfirmation()?.subject).toEqual({
      kind: "ddl",
      statements,
    });
    resolveSafetyConfirmation(true);

    await expect(result).resolves.toEqual({
      kind: "ok",
      value: { appliedStatements: 1, runtimeMs: 6 },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "apply_object_ddl", {
      payload: {
        connectionId: connection.id,
        ops: [...ops],
        confirmed: false,
      },
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "apply_object_ddl", {
      payload: {
        connectionId: connection.id,
        ops: [...ops],
        confirmed: true,
      },
    });
  });

  it("does not retry confirmed DDL in a newer connection lifetime", async () => {
    const statements: DdlStatementSummary[] = [
      {
        index: 0,
        summary: "Drop view public.active_users",
        destructive: true,
        transactional: true,
      },
    ];
    mockedInvoke.mockRejectedValueOnce({
      kind: "policyNeedsConfirmation",
      statements,
    });
    let current = true;

    const result = applyObjectDdlWithSafetyConfirmation(
      {
        connectionId: connection.id,
        ops: [{ op: "dropObject", reference: viewRef, cascade: false }],
      },
      connection,
      () => current,
    );

    await vi.waitFor(() => expect(getSafetyConfirmation()).not.toBeNull());
    current = false;
    resolveSafetyConfirmation(true);

    await expect(result).resolves.toEqual({
      kind: "error",
      error: {
        kind: "connection",
        message: "Connection changed. Regenerate the DDL preview.",
      },
    });
    expect(mockedInvoke).toHaveBeenCalledOnce();
  });
});

describe("object DDL errors and refresh scope", () => {
  it("names a failed statement, applied count, and invalid-index residue", () => {
    expect(
      formatObjectDdlError(
        {
          kind: "database",
          statementIndex: 1,
          code: "42P07",
          message: "relation already exists",
          position: null,
          appliedStatements: 1,
          residue: {
            kind: "invalidIndex",
            schema: "public",
            name: "users_email_idx",
          },
        },
        [{ summary: "Create table" }, { summary: "Create email index" }],
      ),
    ).toBe(
      "Statement 2 (Create email index) failed: relation already exists [42P07]. 1 earlier statement was applied. An invalid index public.users_email_idx was left behind. Drop it before retrying.",
    );

    expect(
      formatObjectDdlError(
        {
          kind: "lockTimeout",
          statementIndex: 0,
          appliedStatements: 2,
        },
        [{ summary: "Drop active users view" }],
      ),
    ).toBe(
      "Statement 1 (Drop active users view) timed out waiting for a database lock. 2 earlier statements were applied.",
    );
  });

  it("reloads the catalog only for catalog-affecting operations", () => {
    expect(
      objectDdlRefreshScope([
        {
          op: "alterSequence",
          schema: "public",
          name: "invoice_number",
          restartWith: null,
          incrementBy: "10",
          minValue: null,
          maxValue: null,
          cycle: null,
          cache: null,
        },
      ]),
    ).toEqual({
      catalog: false,
      references: [
        {
          kind: "sequence",
          schema: "public",
          name: "invoice_number",
          identityArgs: null,
        },
      ],
    });

    expect(
      objectDdlRefreshScope([
        { op: "dropObject", reference: viewRef, cascade: false },
      ]),
    ).toEqual({ catalog: true, references: [viewRef] });
  });
});
