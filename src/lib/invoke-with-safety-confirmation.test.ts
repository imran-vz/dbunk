/* oxlint-disable anti-slop/no-module-mocking anti-slop/no-unknown-parameters -- The test controls the Tauri rejection boundary. */
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";
import type { Connection } from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const connection: Connection = {
  id: "conn-1",
  name: "Staging",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "staging.internal",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "10 ms",
  ssl: true,
  environment: "staging",
  safeMode: "protected",
  readOnly: false,
};

const mockedInvoke = vi.mocked(tauriInvoke);

beforeEach(() => {
  mockedInvoke.mockReset();
  resolveSafetyConfirmation(false);
});

describe("invokeWithSafetyConfirmation", () => {
  it("routes an exact confirm tag and retries with confirmed true", async () => {
    mockedInvoke
      .mockRejectedValueOnce("[policy:confirm] protected write")
      .mockResolvedValueOnce({ runtimeMs: 4 });

    const result = invokeWithSafetyConfirmation<{ runtimeMs: number }>({
      command: "execute_ddl",
      connection,
      payload: { connectionId: connection.id, sql: "ALTER TABLE users" },
    });

    await vi.waitFor(() => expect(getSafetyConfirmation()).not.toBeNull());
    expect(getSafetyConfirmation()?.subject).toEqual({
      kind: "command",
      command: "execute_ddl",
      destructive: true,
    });
    expect(mockedInvoke).toHaveBeenNthCalledWith(1, "execute_ddl", {
      payload: {
        connectionId: connection.id,
        sql: "ALTER TABLE users",
      },
    });
    resolveSafetyConfirmation(true);

    await expect(result).resolves.toEqual({ runtimeMs: 4 });
    expect(mockedInvoke).toHaveBeenNthCalledWith(2, "execute_ddl", {
      payload: {
        connectionId: connection.id,
        sql: "ALTER TABLE users",
        confirmed: true,
      },
    });
  });

  it("classifies known additive commands without destructive typing", async () => {
    mockedInvoke.mockRejectedValueOnce("[policy:confirm] protected write");

    const result = invokeWithSafetyConfirmation({
      command: "insert_row",
      connection,
      payload: { connectionId: connection.id },
    });

    await vi.waitFor(() => expect(getSafetyConfirmation()).not.toBeNull());
    expect(getSafetyConfirmation()?.subject).toEqual({
      kind: "command",
      command: "insert_row",
      destructive: false,
    });
    resolveSafetyConfirmation(false);
    await expect(result).rejects.toThrow("Safety confirmation cancelled.");
  });

  it("surfaces read-only tags without opening a confirmation", async () => {
    mockedInvoke.mockRejectedValueOnce("[policy:read-only] writes disabled");

    await expect(
      invokeWithSafetyConfirmation({
        command: "insert_row",
        connection,
        payload: { connectionId: connection.id },
      }),
    ).rejects.toBe("[policy:read-only] writes disabled");
    expect(getSafetyConfirmation()).toBeNull();
    expect(mockedInvoke).toHaveBeenCalledOnce();
  });
});
