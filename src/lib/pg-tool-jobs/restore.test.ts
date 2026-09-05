import { afterEach, expect, it, vi } from "vitest";

import {
  getSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";

import { pgToolObserver } from "./observer";
import { startReviewedPgRestore } from "./restore";
const payload = {
  connectionId: "c",
  sourcePath: "/tmp/a",
  format: "custom" as const,
  clean: true,
};
const connection = { name: "Local", environment: "development" as const };
const refusal = {
  kind: "policyNeedsConfirmation",
  statements: [{ index: 0, class: "ddl", destructive: true, unbounded: false }],
};
afterEach(() => {
  resolveSafetyConfirmation(false);
  vi.restoreAllMocks();
});
it("retries only the frozen request after deliberate confirmation", async () => {
  const start = vi
    .spyOn(pgToolObserver, "startRestore")
    .mockRejectedValueOnce(refusal)
    .mockResolvedValueOnce({
      jobId: "j",
      connectionId: "c",
      kind: "restore",
      format: "custom",
      fileName: "a",
      phase: "queued",
      startedAt: "now",
      finishedAt: null,
      bytesProcessed: null,
      totalBytes: null,
      toolVersion: null,
      failure: null,
    });
  const mutable = { ...payload };
  const result = startReviewedPgRestore(mutable, connection, () => true);
  await Promise.resolve();
  expect(getSafetyConfirmation()?.subject.kind).toBe("statements");
  mutable.sourcePath = "/tmp/changed";
  resolveSafetyConfirmation(true);
  await result;
  expect(start).toHaveBeenLastCalledWith({ ...payload, confirmed: true });
});
it("rejects a retargeted connection or global fence during confirmation", async () => {
  let current = true;
  const start = vi
    .spyOn(pgToolObserver, "startRestore")
    .mockRejectedValue(refusal);
  const result = startReviewedPgRestore(payload, connection, () => current);
  await Promise.resolve();
  current = false;
  resolveSafetyConfirmation(true);
  await expect(result).rejects.toEqual({ kind: "connectionClosing" });
  expect(start).toHaveBeenCalledTimes(1);
});
it("does not retry a read-only refusal", async () => {
  const start = vi
    .spyOn(pgToolObserver, "startRestore")
    .mockRejectedValue({ kind: "policyBlocked", reason: "Read only" });
  await expect(
    startReviewedPgRestore(payload, connection, () => true),
  ).rejects.toMatchObject({ kind: "policyBlocked" });
  expect(getSafetyConfirmation()).toBeNull();
  expect(start).toHaveBeenCalledTimes(1);
});
