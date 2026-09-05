// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

import { pgTransferClient, type PgTransferJob } from "./client";
import { createPgTransferObserver } from "./observer";

const job: PgTransferJob = {
  jobId: "j",
  connectionId: "c",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  phase: "running",
  startedAt: "2026-09-05T00:00:00Z",
  finishedAt: null,
  totalBytes: 100,
  bytesProcessed: 10,
  rowsProcessed: 2,
  rowsCommitted: null,
  failure: null,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("PostgreSQL transfer observation", () => {
  it("never overlaps polls and follows cancellation with a fresh list", async () => {
    const first = deferred<PgTransferJob[]>();
    const list = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue([{ ...job, phase: "cancelled" }]);
    const observer = createPgTransferObserver({
      ...pgTransferClient,
      list,
      cancel: async () => ({ ...job, phase: "cancelling" }),
    });
    const polling = observer.refresh();
    await observer.cancel(job.jobId);
    expect(observer.store.getState().jobs[0]?.phase).toBe("cancelling");
    first.resolve([job]);
    await polling;
    expect(list).toHaveBeenCalledTimes(2);
    expect(observer.store.getState().jobs[0]?.phase).toBe("cancelled");
  });

  it("reconciles uncertain admission without retrying start", async () => {
    const staleList = deferred<PgTransferJob[]>();
    const startImport = vi.fn().mockRejectedValue(new Error("lost response"));
    const list = vi
      .fn()
      .mockReturnValueOnce(staleList.promise)
      .mockResolvedValueOnce([job]);
    const observer = createPgTransferObserver({
      ...pgTransferClient,
      startImport,
      list,
    });

    const stalePoll = observer.refresh();
    let settled = false;
    const admission = observer
      .startImport({
        inspectionToken: "review-1",
        mapping: [{ sourceIndex: 0, targetColumn: "id" }],
        confirmed: false,
      })
      .finally(() => {
        settled = true;
      });
    void admission.catch(() => undefined);
    await Promise.resolve();
    expect(startImport).toHaveBeenCalledOnce();
    expect(settled).toBe(false);

    staleList.resolve([]);
    await stalePoll;
    await expect(admission).rejects.toThrow("lost response");
    expect(list).toHaveBeenCalledTimes(2);
    expect(observer.store.getState().jobs).toEqual([job]);
  });

  it("keeps polling an import after the table view closes and emits each data impact once", async () => {
    vi.useFakeTimers();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const completed = { ...job, phase: "completed" as const, rowsCommitted: 2 };
    const list = vi.fn().mockResolvedValue([completed]);
    const impact = vi.fn();
    const observer = createPgTransferObserver({ ...pgTransferClient, list });
    const stop = observer.mount(impact);
    const leaveView = observer.consume();
    leaveView();

    await observer.refresh();
    await observer.refresh();
    expect(impact).toHaveBeenCalledOnce();
    expect(impact).toHaveBeenCalledWith(completed);
    stop();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("treats outcomeUnknown as terminal and emits its data impact", async () => {
    const unknown = {
      ...job,
      phase: "outcomeUnknown" as const,
      failure: { kind: "outcomeUnknown" as const },
    };
    const impact = vi.fn();
    const observer = createPgTransferObserver({
      ...pgTransferClient,
      list: async () => [unknown],
    });
    const stop = observer.mount(impact);
    await observer.refresh();
    expect(impact).toHaveBeenCalledWith(unknown);
    expect(observer.store.getState().jobs[0]?.phase).toBe("outcomeUnknown");
    stop();
  });
});
