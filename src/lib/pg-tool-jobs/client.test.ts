/* oxlint-disable anti-slop/no-module-mocking -- Tests verify the real IPC serialization boundary. */
import { beforeEach, expect, it, vi } from "vitest";

import { tauriInvoke } from "@/lib/tauri";

import { decodePgToolError, pgToolClient, type PgToolJob } from "./client";
vi.mock("@/lib/tauri", () => ({ tauriInvoke: vi.fn() }));
const job: PgToolJob = {
  jobId: "j",
  connectionId: "c",
  kind: "backup",
  format: "plain",
  fileName: "out.sql",
  phase: "queued",
  startedAt: "now",
  finishedAt: null,
  bytesProcessed: null,
  totalBytes: null,
  toolVersion: null,
  failure: null,
};
beforeEach(() => vi.mocked(tauriInvoke).mockReset());
it("uses the six distinct command envelopes and preserves explicit format", async () => {
  vi.mocked(tauriInvoke).mockResolvedValue(job);
  const payload = {
    connectionId: "c",
    destinationPath: "/tmp/out.sql",
    format: "plain" as const,
    clean: true,
    scope: { kind: "schema" as const, schema: "Weird.*Schema" },
  };
  await pgToolClient.startBackup(payload);
  expect(tauriInvoke).toHaveBeenLastCalledWith("start_pg_backup", { payload });
  const restore = {
    connectionId: "c",
    sourcePath: "/tmp/not-sql.dump",
    format: "plain" as const,
    clean: false,
    confirmed: false,
  };
  await pgToolClient.startRestore(restore);
  expect(tauriInvoke).toHaveBeenLastCalledWith("start_pg_restore", {
    payload: restore,
  });
  await pgToolClient.get("j");
  expect(tauriInvoke).toHaveBeenLastCalledWith("get_pg_tool_job", {
    jobId: "j",
  });
  await pgToolClient.cancel("j");
  expect(tauriInvoke).toHaveBeenLastCalledWith("cancel_pg_tool_job", {
    jobId: "j",
  });
  await pgToolClient.release("j");
  expect(tauriInvoke).toHaveBeenLastCalledWith("release_pg_tool_job", {
    jobId: "j",
  });
  vi.mocked(tauriInvoke).mockResolvedValue([job]);
  await pgToolClient.list();
  expect(tauriInvoke).toHaveBeenLastCalledWith("list_pg_tool_jobs", {
    connectionId: null,
  });
});
it("decodes typed refusals and withholds unknown rejection contents", () => {
  expect(
    decodePgToolError({
      kind: "policyNeedsConfirmation",
      statements: [
        { index: 0, class: "ddl", destructive: true, unbounded: false },
      ],
    }).kind,
  ).toBe("policyNeedsConfirmation");
  expect(
    decodePgToolError({
      kind: "policyNeedsConfirmation",
      statements: [{ sql: "password" }],
    }),
  ).toEqual({ kind: "transport" });
  expect(decodePgToolError(new Error("secret path"))).toEqual({
    kind: "transport",
  });
  expect(decodePgToolError({ kind: "timeout", operation: "reap" })).toEqual({
    kind: "timeout",
    operation: "reap",
  });
});
it("returns cancellation without starting any job", async () => {
  vi.mocked(tauriInvoke).mockResolvedValue(null);
  expect(await pgToolClient.pick("restore", "custom")).toBeNull();
  expect(tauriInvoke).toHaveBeenCalledTimes(1);
  expect(tauriInvoke).toHaveBeenCalledWith("pick_pg_tool_file", {
    kind: "restore",
    format: "custom",
  });
});
