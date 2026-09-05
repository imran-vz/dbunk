/* oxlint-disable anti-slop/no-module-mocking -- Tests verify the real IPC serialization boundary. */
import { beforeEach, expect, it, vi } from "vitest";

import { tauriInvoke } from "@/lib/tauri";

import {
  decodePgTransferError,
  DEFAULT_PG_CSV_OPTIONS,
  formatPgTransferError,
  pgTransferClient,
  type PgTransferInspection,
  type PgTransferJob,
} from "./client";

vi.mock("@/lib/tauri", () => ({ tauriInvoke: vi.fn() }));

const inspection: PgTransferInspection = {
  inspectionToken: "review-1",
  connectionId: "c",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  totalBytes: 20,
  sourceColumns: [{ index: 0, name: "id" }],
  targetColumns: [
    {
      name: "id",
      dataType: "integer",
      nullable: false,
      hasDefault: false,
      generated: false,
      identity: false,
    },
  ],
  sampleRows: [["1"]],
  sampleTruncated: false,
  options: DEFAULT_PG_CSV_OPTIONS,
};
const job: PgTransferJob = {
  jobId: "j",
  connectionId: "c",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  phase: "preparing",
  startedAt: "2026-09-05T00:00:00Z",
  finishedAt: null,
  totalBytes: 20,
  bytesProcessed: 0,
  rowsProcessed: null,
  rowsCommitted: null,
  failure: null,
};

beforeEach(() => vi.mocked(tauriInvoke).mockReset());

it("uses typed transfer command envelopes", async () => {
  vi.mocked(tauriInvoke).mockResolvedValueOnce(inspection);
  const inspectPayload = {
    connectionId: "c",
    schema: "public",
    table: "users",
    direction: "import" as const,
    sourcePath: "/tmp/users.csv",
    options: DEFAULT_PG_CSV_OPTIONS,
  };
  await pgTransferClient.inspect(inspectPayload);
  expect(tauriInvoke).toHaveBeenLastCalledWith("inspect_pg_transfer", {
    payload: inspectPayload,
  });

  vi.mocked(tauriInvoke).mockResolvedValueOnce(job);
  const startPayload = {
    inspectionToken: "review-1",
    mapping: [{ sourceIndex: 0, targetColumn: "id" }],
    confirmed: false,
  };
  await pgTransferClient.startImport(startPayload);
  expect(tauriInvoke).toHaveBeenLastCalledWith("start_pg_csv_import", {
    payload: startPayload,
  });

  vi.mocked(tauriInvoke).mockResolvedValueOnce([job]);
  await pgTransferClient.list("c");
  expect(tauriInvoke).toHaveBeenLastCalledWith("list_pg_transfer_jobs", {
    connectionId: "c",
  });
});

it("rejects malformed counters and withholds unknown rejection contents", async () => {
  vi.mocked(tauriInvoke).mockResolvedValue({
    ...job,
    bytesProcessed: Number.MAX_SAFE_INTEGER + 1,
  });
  await expect(pgTransferClient.get("j")).rejects.toThrow();
  expect(decodePgTransferError(new Error("/secret/source.csv"))).toEqual({
    kind: "transport",
  });
  expect(
    decodePgTransferError({
      kind: "policyNeedsConfirmation",
      statements: [{ sql: "secret" }],
    }),
  ).toEqual({ kind: "transport" });
  expect(
    decodePgTransferError({ kind: "csv", record: 3, column: 1, reason: "bad" }),
  ).toEqual({ kind: "csv", record: 3, column: 1, reason: "bad" });
});

it("formats native CSV columns as the one-based positions in the wire contract", () => {
  expect(
    formatPgTransferError({
      kind: "csv",
      record: 3,
      column: 1,
      reason: "bad",
    }),
  ).toBe("CSV record 3, column 1: bad");
});

it("returns picker cancellation without inspecting or starting", async () => {
  vi.mocked(tauriInvoke).mockResolvedValue(null);
  expect(await pgTransferClient.pick("import")).toBeNull();
  expect(tauriInvoke).toHaveBeenCalledOnce();
  expect(tauriInvoke).toHaveBeenCalledWith("pick_pg_transfer_file", {
    direction: "import",
  });
});
