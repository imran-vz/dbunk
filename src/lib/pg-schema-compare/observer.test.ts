// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";

import { createSchemaCompareRequest, schemaCompareClient } from "./client";
import { createSchemaCompareObserver } from "./observer";
import type { SchemaCompareStatus } from "./protocol";

const source = { connectionId: "a", schema: "public" };
const target = { connectionId: "b", schema: "public" };
const payload = createSchemaCompareRequest(source, target);
const job: SchemaCompareStatus = {
  jobId: "job",
  requestId: payload.requestId,
  source,
  target,
  sourceObjects: 3,
  targetObjects: 2,
  phase: "comparing",
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("schema comparison observation", () => {
  it("holds a lost start response until a later list resolves it by request id, without a second start", async () => {
    const staleList = deferred<SchemaCompareStatus[]>();
    const start = vi.fn().mockRejectedValue(new TypeError("lost response"));
    const list = vi
      .fn()
      .mockReturnValueOnce(staleList.promise)
      .mockResolvedValueOnce([job]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      start,
      list,
    });
    const stalePoll = observer.refresh();
    let settled = false;
    const admission = observer.start(payload).finally(() => {
      settled = true;
    });
    void admission.catch(() => undefined);
    await Promise.resolve();
    expect(start).toHaveBeenCalledTimes(1);
    expect(settled).toBe(false);
    expect(observer.store.getState().uncertainRequestId).toBe(
      payload.requestId,
    );

    // A list issued before the failure cannot resolve the uncertainty.
    staleList.resolve([]);
    await stalePoll;
    await expect(admission).rejects.toThrow("lost response");
    expect(list).toHaveBeenCalledTimes(2);
    expect(start).toHaveBeenCalledTimes(1);
    const state = observer.store.getState();
    expect(state.uncertainRequestId).toBeNull();
    expect(state.jobs.find((j) => j.requestId === payload.requestId)).toEqual(
      job,
    );
  });

  it("keeps admission uncertain while lists fail and clears it only after one succeeds", async () => {
    vi.useFakeTimers();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const start = vi.fn().mockRejectedValue(new TypeError("lost response"));
    const list = vi
      .fn()
      .mockRejectedValueOnce("down")
      .mockRejectedValueOnce("still down")
      .mockResolvedValue([]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      start,
      list,
    });
    const unmount = observer.mount();
    try {
      await vi.advanceTimersByTimeAsync(0);
      await expect(observer.start(payload)).rejects.toThrow("lost response");
      expect(observer.store.getState().uncertainRequestId).toBe(
        payload.requestId,
      );
      expect(observer.store.getState().error).toEqual({ kind: "transport" });
      // Backoff doubles from one second and keeps polling with no consumers.
      await vi.advanceTimersByTimeAsync(3999);
      expect(observer.store.getState().uncertainRequestId).toBe(
        payload.requestId,
      );
      await vi.advanceTimersByTimeAsync(1);
      expect(observer.store.getState().uncertainRequestId).toBeNull();
      expect(observer.store.getState().error).toBeNull();
      expect(start).toHaveBeenCalledTimes(1);
      // Nothing active and no consumers: polling stops.
      await vi.advanceTimersByTimeAsync(20_000);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      unmount();
    }
  });

  it("does not treat a native rejection as uncertain admission", async () => {
    const start = vi.fn().mockRejectedValue({ kind: "busy" });
    const list = vi.fn().mockResolvedValue([job]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      start,
      list,
    });
    await expect(observer.start(payload)).rejects.toEqual({ kind: "busy" });
    expect(observer.store.getState().uncertainRequestId).toBeNull();
  });

  it("shows cancelling until the backend reports the terminal state, even when that state is completed", async () => {
    const first = deferred<SchemaCompareStatus[]>();
    const list = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockResolvedValue([{ ...job, phase: "completed", resultId: "r" }]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
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

  it("retains a record whose release failed and drops one the backend no longer has", async () => {
    const list = vi.fn().mockResolvedValue([]);
    const release = vi
      .fn()
      .mockRejectedValueOnce({ kind: "busy" })
      .mockRejectedValueOnce({ kind: "unavailable" });
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      list,
      release,
    });
    const terminal: SchemaCompareStatus = { ...job, phase: "cancelled" };
    observer.store.setState({ jobs: [terminal] });
    await expect(observer.release(job.jobId)).rejects.toEqual({
      kind: "busy",
    });
    expect(observer.store.getState().jobs).toEqual([terminal]);
    await observer.release(job.jobId);
    expect(observer.store.getState().jobs).toEqual([]);
  });

  it("does not resurrect a dismissed record from an earlier list", async () => {
    const old = deferred<SchemaCompareStatus[]>();
    const list = vi.fn().mockReturnValueOnce(old.promise).mockResolvedValue([]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      list,
      release: async () => {},
    });
    const terminal: SchemaCompareStatus = { ...job, phase: "cancelled" };
    observer.store.setState({ jobs: [terminal] });
    const read = observer.refresh();
    await observer.release(job.jobId);
    old.resolve([terminal]);
    await read;
    expect(observer.store.getState().jobs).toEqual([]);
  });

  it("pauses polling while hidden and reconciles immediately on return", async () => {
    vi.useFakeTimers();
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    const list = vi.fn().mockResolvedValue([job]);
    const observer = createSchemaCompareObserver({
      ...schemaCompareClient,
      list,
    });
    const unmount = observer.mount();
    const leave = observer.consume();
    await vi.advanceTimersByTimeAsync(0);
    const calls = list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(1000);
    expect(list.mock.calls.length).toBe(calls + 1);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
    const hiddenCalls = list.mock.calls.length;
    await vi.advanceTimersByTimeAsync(20_000);
    expect(list.mock.calls.length).toBe(hiddenCalls);
    list.mockResolvedValue([]);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(0);
    expect(list.mock.calls.length).toBe(hiddenCalls + 1);
    expect(observer.store.getState().jobs).toEqual([]);
    leave();
    unmount();
    expect(vi.getTimerCount()).toBe(0);
  });
});
