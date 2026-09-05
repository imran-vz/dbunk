import { afterEach, expect, it, vi } from "vitest";

import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";

import { pgTransferObserver } from "./observer";
import { startReviewedPgCsvImport } from "./start-import";

const payload = {
  inspectionToken: "review-1",
  mapping: [{ sourceIndex: 1, targetColumn: "email" }],
};
const connection = { name: "Local", environment: "production" as const };
const refusal = {
  kind: "policyNeedsConfirmation",
  statements: [
    { index: 0, class: "dml", destructive: false, unbounded: false },
  ],
};
const job = {
  jobId: "j",
  connectionId: "c",
  schema: "public",
  table: "users",
  direction: "import" as const,
  fileName: "users.csv",
  phase: "preparing" as const,
  startedAt: "now",
  finishedAt: null,
  totalBytes: 1,
  bytesProcessed: 0,
  rowsProcessed: null,
  rowsCommitted: null,
  failure: null,
};

afterEach(() => {
  resolveSafetyConfirmation(false);
  vi.restoreAllMocks();
});

it("retries only the frozen source-index mapping after confirmation", async () => {
  const start = vi
    .spyOn(pgTransferObserver, "startImport")
    .mockRejectedValueOnce(refusal)
    .mockResolvedValueOnce(job);
  const mutable = {
    ...payload,
    mapping: payload.mapping.map((entry) => ({ ...entry })),
  };
  const result = startReviewedPgCsvImport(mutable, connection, () => true);
  await Promise.resolve();
  expect(getSafetyConfirmation()?.subject.kind).toBe("statements");
  mutable.mapping[0]!.sourceIndex = 99;
  mutable.mapping[0]!.targetColumn = "changed";
  resolveSafetyConfirmation(true);
  await result;
  expect(start).toHaveBeenLastCalledWith({ ...payload, confirmed: true });
});

it("rejects a stale form or connection after confirmation", async () => {
  let current = true;
  const start = vi
    .spyOn(pgTransferObserver, "startImport")
    .mockRejectedValue(refusal);
  const result = startReviewedPgCsvImport(payload, connection, () => current);
  await Promise.resolve();
  current = false;
  resolveSafetyConfirmation(true);
  await expect(result).rejects.toEqual({ kind: "connectionClosing" });
  expect(start).toHaveBeenCalledOnce();
});
