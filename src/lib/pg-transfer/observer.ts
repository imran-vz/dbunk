import { createStore } from "zustand/vanilla";

import {
  decodePgTransferError,
  isPgTransferJobActive,
  pgTransferClient,
  type PgTransferFailure,
  type PgTransferJob,
} from "./client";

export type PgTransferObservation = {
  jobs: PgTransferJob[];
  error: PgTransferFailure | null;
  refreshing: boolean;
  observedAt: number | null;
};

/** App-owned observation keeps native transfers alive and visible across table tabs. */
export function createPgTransferObserver(client = pgTransferClient) {
  const store = createStore<PgTransferObservation>(() => ({
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
  let uncertainAdmission: number | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let polling: Promise<void> | undefined;
  let refreshAgain = false;
  let onImportSettled: (job: PgTransferJob) => void = () => {};
  const settledImports = new Set<string>();

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
        !store.getState().jobs.some(isPgTransferJobActive))
    ) {
      return;
    }
    timer = setTimeout(
      () => void refresh(),
      Math.min(1000 * 2 ** failures, 15_000),
    );
  }

  function accept(jobs: PgTransferJob[], request: number) {
    if (request < applied) return;
    applied = request;
    const retained = [
      ...jobs.filter(isPgTransferJobActive),
      ...jobs
        .filter((job) => !isPgTransferJobActive(job))
        .sort((a, b) => b.startedAt.localeCompare(a.startedAt))
        .slice(0, 32),
    ];
    for (const id of settledImports) {
      if (!retained.some((job) => job.jobId === id)) settledImports.delete(id);
    }
    store.setState({ jobs: retained, error: null, observedAt: Date.now() });
    for (const job of retained) {
      if (
        job.direction === "import" &&
        (job.phase === "completed" || job.phase === "outcomeUnknown") &&
        !settledImports.has(job.jobId)
      ) {
        settledImports.add(job.jobId);
        onImportSettled(job);
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
            if (request >= applied) {
              store.setState({ error: decodePgTransferError(error) });
            }
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
    run: () => Promise<PgTransferJob>,
    awaitUncertainAdmission = false,
  ) {
    const request = ++sequence;
    try {
      const job = await run();
      accept(
        [
          ...store.getState().jobs.filter((item) => item.jobId !== job.jobId),
          job,
        ],
        request,
      );
      void refresh();
      return job;
    } catch (error) {
      const uncertain =
        awaitUncertainAdmission &&
        decodePgTransferError(error).kind === "transport";
      if (uncertain) uncertainAdmission = ++sequence;
      const reconciliation = refresh();
      if (uncertain) await reconciliation;
      throw error;
    }
  }

  return {
    store,
    refresh,
    startImport: (payload: Parameters<typeof client.startImport>[0]) =>
      mutate(() => client.startImport(payload), true),
    startExport: (payload: Parameters<typeof client.startExport>[0]) =>
      mutate(() => client.startExport(payload), true),
    cancel: (jobId: string) => mutate(() => client.cancel(jobId)),
    async release(jobId: string) {
      ++sequence;
      try {
        await client.release(jobId);
      } catch (error) {
        if (decodePgTransferError(error).kind !== "jobNotFound") throw error;
      }
      applied = ++sequence;
      store.setState({
        jobs: store.getState().jobs.filter((job) => job.jobId !== jobId),
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
    mount(completion: (job: PgTransferJob) => void) {
      mounted = true;
      onImportSettled = completion;
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

export const pgTransferObserver = createPgTransferObserver();
