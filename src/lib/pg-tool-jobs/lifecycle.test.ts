/* oxlint-disable anti-slop/no-module-mocking -- Isolate native availability and user confirmation at their external boundaries. */
import { afterEach, expect, it, vi } from "vitest";

import { requestConfirm } from "@/lib/confirm";
import { pgTransferClient } from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";

import { pgToolClient } from "./client";
import { pgToolReview, preparePgToolFence } from "./lifecycle";
import { pgToolObserver } from "./observer";

vi.mock("@/lib/tauri", () => ({ isTauri: () => true }));
vi.mock("@/lib/confirm", () => ({ requestConfirm: vi.fn() }));
afterEach(() => {
  vi.restoreAllMocks();
  pgToolReview.setState({ revision: 0, closing: 0 });
});
it("invalidates reviews before inspection and preserves edits when inspection fails and continuation is declined", async () => {
  vi.spyOn(pgToolObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgTransferObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgToolClient, "list").mockRejectedValue(new Error("transport"));
  vi.spyOn(pgTransferClient, "list").mockResolvedValue([]);
  vi.mocked(requestConfirm).mockResolvedValue(false);
  const pending = preparePgToolFence("Save connection", "c");
  expect(pgToolReview.getState()).toEqual({ revision: 1, closing: 1 });
  expect(await pending).toBeNull();
  expect(requestConfirm).toHaveBeenCalledWith(
    expect.objectContaining({ cancelLabel: "Keep editing" }),
  );
  expect(pgToolReview.getState()).toEqual({ revision: 2, closing: 0 });
});
it("holds nested global fences until all actions finish", async () => {
  vi.spyOn(pgToolObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgTransferObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgToolClient, "list").mockResolvedValue([]);
  vi.spyOn(pgTransferClient, "list").mockResolvedValue([]);
  const first = await preparePgToolFence("Save connection", "c");
  const second = await preparePgToolFence("Change credential storage");
  expect(pgToolReview.getState().closing).toBe(2);
  first?.();
  first?.();
  expect(pgToolReview.getState().closing).toBe(1);
  second?.();
  expect(pgToolReview.getState()).toEqual({ revision: 4, closing: 0 });
});
it("warns about active transfer jobs alongside backup and restore", async () => {
  vi.spyOn(pgToolObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgTransferObserver, "refresh").mockResolvedValue();
  vi.spyOn(pgToolClient, "list").mockResolvedValue([]);
  vi.spyOn(pgTransferClient, "list").mockResolvedValue([
    {
      jobId: "transfer-1",
      connectionId: "c",
      schema: "public",
      table: "users",
      direction: "import",
      fileName: "users.csv",
      phase: "running",
      startedAt: "2026-09-05T00:00:00Z",
      finishedAt: null,
      totalBytes: 10,
      bytesProcessed: 5,
      rowsProcessed: 1,
      rowsCommitted: null,
      failure: null,
    },
  ]);
  vi.mocked(requestConfirm).mockResolvedValue(false);

  expect(await preparePgToolFence("Disconnect", "c")).toBeNull();
  expect(requestConfirm).toHaveBeenCalledWith(
    expect.objectContaining({
      message: expect.stringContaining("1 CSV transfer"),
    }),
  );
});
