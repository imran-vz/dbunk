import { createStore } from "zustand/vanilla";

import {
  decodePgToolError,
  isPgToolJobActive,
  pgToolClient,
  type PgToolFailure,
  type PgToolJob,
} from "./client";

export type PgToolObservation = {
  jobs: PgToolJob[];
  error: PgToolFailure | null;
  refreshing: boolean;
  observedAt: number | null;
};

/** One app-owned observer. Views subscribe; leaving a view never stops native work. */
export function createPgToolObserver(client = pgToolClient) {
  const store = createStore<PgToolObservation>(() => ({
    jobs: [],
    error: null,
    refreshing: false,
    observedAt: null,
  }));
  let sequence = 0;
  let applied = 0;
  let consumers = 0;
  let mounted = false;
  let visible = true;
  let failures = 0;
  // A lost start response may hide native work even with no subscribed views.
  // Only an accepted full list issued after that failure resolves uncertainty.
  let uncertainAdmission: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let polling: Promise<void> | undefined;
  let refreshAgain = false;
  let onRestoreComplete: (job: PgToolJob) => void = () => {};
  const completed = new Set<string>();

  function clearTimer() {
    clearTimeout(timer);
    timer = undefined;
  }
  function schedule() {
    clearTimer();
    if (
      !mounted ||
      !visible ||
      (!consumers &&
        uncertainAdmission === null &&
        !store.getState().jobs.some(isPgToolJobActive))
    )
      return;
    timer = setTimeout(
      () => {
        void refresh();
      },
      Math.min(1000 * 2 ** failures, 15000),
    );
  }
  function accept(jobs: PgToolJob[], request: number) {
    if (request < applied) return;
    applied = request;
    // Active records are never evicted; the backend admits at most four.
    const retained = [
      ...jobs.filter(isPgToolJobActive),
      ...jobs
        .filter((j) => !isPgToolJobActive(j))
        .sort((a, b) => b.startedAt.localeCompare(a.startedAt))
        .slice(0, 32),
    ];
    for (const id of completed)
      if (!retained.some((j) => j.jobId === id)) completed.delete(id);
    store.setState({ jobs: retained, error: null, observedAt: Date.now() });
    for (const job of retained) {
      if (
        job.kind === "restore" &&
        job.phase === "completed" &&
        !completed.has(job.jobId)
      ) {
        completed.add(job.jobId);
        onRestoreComplete(job);
      }
    }
  }
  function refresh(): Promise<void> {
    if (polling) {
      refreshAgain = true;
      return polling;
    }
    clearTimer();
    store.setState({ refreshing: true });
    polling = (async () => {
      try {
        do {
          refreshAgain = false;
          const request = ++sequence;
          try {
            const jobs = await client.list();
            if (
              request >= applied &&
              uncertainAdmission !== null &&
              request > uncertainAdmission
            ) {
              uncertainAdmission = null;
            }
            accept(jobs, request);
            failures = 0;
          } catch (error) {
            failures = Math.min(failures + 1, 4);
            if (request >= applied)
              store.setState({ error: decodePgToolError(error) });
          }
        } while (refreshAgain);
      } finally {
        polling = undefined;
        store.setState({ refreshing: false });
        schedule();
      }
    })();
    return polling;
  }
  async function mutate(
    run: () => Promise<PgToolJob>,
    awaitUncertainAdmission = false,
  ) {
    const request = ++sequence;
    try {
      const job = await run();
      accept(
        [...store.getState().jobs.filter((j) => j.jobId !== job.jobId), job],
        request,
      );
      void refresh();
      return job;
    } catch (error) {
      const uncertain =
        awaitUncertainAdmission &&
        decodePgToolError(error).kind === "transport";
      if (uncertain) uncertainAdmission = ++sequence;
      const reconciliation = refresh();
      // A transport failure may have lost an accepted start response. Hold the
      // caller's lock through the first reconciliation attempt. Failed lists
      // keep polling with backoff after the caller leaves; never retry start.
      if (uncertain) await reconciliation;
      throw error;
    }
  }
  return {
    store,
    refresh,
    startBackup: (payload: Parameters<typeof client.startBackup>[0]) =>
      mutate(() => client.startBackup(payload), true),
    startRestore: (payload: Parameters<typeof client.startRestore>[0]) =>
      mutate(() => client.startRestore(payload), true),
    cancel: (jobId: string) => mutate(() => client.cancel(jobId)),
    async release(jobId: string) {
      ++sequence;
      try {
        await client.release(jobId);
      } catch (error) {
        if (decodePgToolError(error).kind !== "jobNotFound") throw error;
      }
      // Invalidate observations issued before dismissal, even if another mutation finished.
      applied = ++sequence;
      store.setState({
        jobs: store.getState().jobs.filter((j) => j.jobId !== jobId),
      });
      void refresh();
    },
    consume() {
      consumers++;
      if (mounted && visible) void refresh();
      return () => {
        consumers--;
        schedule();
      };
    },
    mount(completion: (job: PgToolJob) => void) {
      mounted = true;
      onRestoreComplete = completion;
      visible = document.visibilityState !== "hidden";
      const visibilityChanged = () => {
        visible = document.visibilityState !== "hidden";
        if (visible) void refresh();
        else clearTimer();
      };
      document.addEventListener("visibilitychange", visibilityChanged);
      if (visible) void refresh();
      return () => {
        mounted = false;
        clearTimer();
        document.removeEventListener("visibilitychange", visibilityChanged);
      };
    },
  };
}
export const pgToolObserver = createPgToolObserver();
