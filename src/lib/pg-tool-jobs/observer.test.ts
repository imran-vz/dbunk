// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

import { pgToolClient, type PgToolJob } from "./client";
import { createPgToolObserver } from "./observer";

const job: PgToolJob = {
  jobId: "j",
  connectionId: "c",
  kind: "restore",
  format: "custom",
  fileName: "source.dump",
  phase: "running",
  startedAt: "2026-09-05T00:00:00Z",
  finishedAt: null,
  bytesProcessed: null,
  totalBytes: 10,
  toolVersion: null,
  failure: null,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("PostgreSQL tool observation", () => {
  it("never overlaps polls and follows cancellation with a fresh list", async () => {
    const first = deferred<PgToolJob[]>();
    const list = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue([{ ...job, phase: "completed" }]);
    const observer = createPgToolObserver({
      ...pgToolClient,
      list,
      cancel: async () => ({ ...job, phase: "cancelling" }),
    });
    const polling = observer.refresh();
    await observer.cancel(job.jobId);
    expect(observer.store.getState().jobs[0]?.phase).toBe("cancelling");
    first.resolve([job]);
    await polling;
    expect(list).toHaveBeenCalledTimes(2);
    expect(observer.store.getState().jobs[0]?.phase).toBe("completed");
  });
  it("holds uncertain admission until a fresh list completes without retrying start", async () => {
    const staleList = deferred<PgToolJob[]>();
    const startBackup = vi.fn().mockRejectedValue(new Error("lost response"));
    const list = vi
      .fn()
      .mockReturnValueOnce(staleList.promise)
      .mockResolvedValueOnce([job]);
    const observer = createPgToolObserver({
      ...pgToolClient,
      startBackup,
      list,
    });

    const stalePoll = observer.refresh();
    let settled = false;
    const admission = observer
      .startBackup({
        connectionId: "c",
        destinationPath: "/tmp/archive",
        format: "custom",
        clean: false,
        scope: { kind: "database" },
      })
      .finally(() => {
        settled = true;
      });
    void admission.catch(() => undefined);
    await Promise.resolve();

    expect(startBackup).toHaveBeenCalledTimes(1);
    expect(list).toHaveBeenCalledTimes(1);
    expect(settled).toBe(false);

    staleList.resolve([]);
    await stalePoll;
    await expect(admission).rejects.toThrow("lost response");

    expect(list).toHaveBeenCalledTimes(2);
    expect(observer.store.getState().jobs).toEqual([job]);
  });
  it.each([
    {
      outcome: "completed restore",
      jobs: [{ ...job, phase: "completed" as const }],
    },
    { outcome: "no admitted job", jobs: [] },
  ])(
    "reconciles $outcome after the view closes and transport recovers",
    async ({ jobs }) => {
      vi.useFakeTimers();
      vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
      const list = vi.fn().mockResolvedValue([]);
      const startRestore = vi
        .fn()
        .mockRejectedValue(new Error("lost response"));
      const completed = vi.fn();
      const observer = createPgToolObserver({
        ...pgToolClient,
        list,
        startRestore,
      });
      const leaveView = observer.consume();
      const unmount = observer.mount(completed);
      try {
        await observer.refresh();
        // This successful list predates the uncertain admission and cannot resolve it.
        const staleList = deferred<PgToolJob[]>();
        list
          .mockReturnValueOnce(staleList.promise)
          .mockRejectedValueOnce(new Error("observation unavailable"))
          .mockRejectedValueOnce(new Error("observation still unavailable"))
          .mockResolvedValue(jobs);
        const stalePoll = observer.refresh();
        const admission = observer.startRestore({
          connectionId: "c",
          sourcePath: "/tmp/archive",
          format: "custom",
          clean: false,
          confirmed: false,
        });
        const rejection = expect(admission).rejects.toThrow("lost response");
        leaveView();
        await Promise.resolve();
        staleList.resolve([]);
        await stalePoll;
        await rejection;
        expect(observer.store.getState().error).toEqual({ kind: "transport" });

        const callsAfterFailure = list.mock.calls.length;
        await vi.advanceTimersByTimeAsync(1999);
        expect(list).toHaveBeenCalledTimes(callsAfterFailure);
        await vi.advanceTimersByTimeAsync(1);
        expect(list).toHaveBeenCalledTimes(callsAfterFailure + 1);
        expect(completed).not.toHaveBeenCalled();
        await vi.advanceTimersByTimeAsync(4000);

        expect(observer.store.getState().jobs).toEqual(jobs);
        expect(observer.store.getState().error).toBeNull();
        expect(completed).toHaveBeenCalledTimes(jobs.length);
        if (jobs.length) expect(completed).toHaveBeenCalledWith(jobs[0]);
        expect(startRestore).toHaveBeenCalledTimes(1);
        expect(vi.getTimerCount()).toBe(0);
      } finally {
        unmount();
      }
    },
  );
  it("observes completion after cancel once, expires history, and pauses hidden polling", async () => {
    vi.useFakeTimers();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const list = vi.fn().mockResolvedValue([{ ...job, phase: "completed" }]);
    const completed = vi.fn();
    const observer = createPgToolObserver({ ...pgToolClient, list });
    const unmount = observer.mount(completed),
      release = observer.consume();
    await observer.refresh();
    await observer.refresh();
    expect(completed).toHaveBeenCalledTimes(1);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    const calls = list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(20000);
    expect(list).toHaveBeenCalledTimes(calls);
    list.mockResolvedValue([]);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await observer.refresh();
    expect(observer.store.getState().jobs).toEqual([]);
    release();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
  it("does not resurrect a dismissed terminal record from an earlier list", async () => {
    const old = deferred<PgToolJob[]>();
    const list = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue([]);
    const observer = createPgToolObserver({
      ...pgToolClient,
      list,
      release: async () => {},
    });
    observer.store.setState({ jobs: [{ ...job, phase: "completed" }] });
    const read = observer.refresh();
    await observer.release("j");
    old.resolve([{ ...job, phase: "completed" }]);
    await read;
    expect(list).toHaveBeenCalledTimes(2);
    expect(observer.store.getState().jobs).toEqual([]);
  });
});
