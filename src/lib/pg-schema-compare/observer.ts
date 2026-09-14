import { createStore } from "zustand/vanilla";

import { schemaCompareClient } from "./client";
import {
  decodeSchemaCompareFailure,
  isSchemaCompareActive,
  type SchemaCompareFailure,
} from "./failure";
import type {
  SchemaCompareStartRequest,
  SchemaCompareStatus,
} from "./protocol";

export type SchemaCompareObservation = {
  /** Native job records, at most the backend's four, in native order. */
  jobs: SchemaCompareStatus[];
  error: SchemaCompareFailure | null;
  refreshing: boolean;
  observedAt: number | null;
  /**
   * A start whose response was lost or unreadable. Only a full list issued
   * after that failure can say whether the request was admitted. The caller
   * keeps the exact request; it is never replayed with a fresh ID here.
   */
  uncertainRequestId: string | null;
};

/**
 * One application-owned observer of native comparison jobs. Views subscribe
 * while mounted; leaving a view never cancels native work. Polling is serial,
 * runs only while visible with consumers, active jobs or uncertain admission,
 * and backs off from one second to fifteen after failed lists.
 */
export function createSchemaCompareObserver(client = schemaCompareClient) {
  const store = createStore<SchemaCompareObservation>(() => ({
    jobs: [],
    error: null,
    refreshing: false,
    observedAt: null,
    uncertainRequestId: null,
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
        !store.getState().jobs.some(isSchemaCompareActive))
    )
      return;
    timer = setTimeout(
      () => {
        void refresh();
      },
      Math.min(1000 * 2 ** failures, 15000),
    );
  }
  function accept(jobs: SchemaCompareStatus[], request: number) {
    if (request < applied) return;
    applied = request;
    store.setState({ jobs, error: null, observedAt: Date.now() });
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
              store.setState({ uncertainRequestId: null });
            }
            accept(jobs, request);
            failures = 0;
          } catch (error) {
            failures = Math.min(failures + 1, 4);
            if (request >= applied)
              store.setState({ error: decodeSchemaCompareFailure(error) });
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
  function merge(status: SchemaCompareStatus, request: number) {
    accept(
      [
        ...store.getState().jobs.filter((j) => j.jobId !== status.jobId),
        status,
      ],
      request,
    );
  }
  return {
    store,
    refresh,
    /**
     * Admits exactly one request. A lost or unreadable response holds the
     * caller through the first reconciliation; the request stays uncertain
     * until a later list succeeds, and start is never retried here.
     */
    async start(payload: SchemaCompareStartRequest) {
      const request = ++sequence;
      try {
        const status = await client.start(payload);
        merge(status, request);
        void refresh();
        return status;
      } catch (error) {
        const failure = decodeSchemaCompareFailure(error);
        const uncertain =
          failure.kind === "transport" || failure.kind === "invalidResponse";
        if (uncertain) {
          uncertainAdmission = ++sequence;
          store.setState({ uncertainRequestId: payload.requestId });
        }
        const reconciliation = refresh();
        if (uncertain) await reconciliation;
        throw error;
      }
    },
    async cancel(jobId: string) {
      const request = ++sequence;
      try {
        const status = await client.cancel(jobId);
        merge(status, request);
        return status;
      } finally {
        void refresh();
      }
    },
    /** Releases a terminal job. Failure keeps the record so the view can retry. */
    async release(jobId: string) {
      ++sequence;
      try {
        await client.release(jobId);
      } catch (error) {
        if (decodeSchemaCompareFailure(error).kind !== "unavailable")
          throw error;
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
    mount() {
      mounted = true;
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
export type SchemaCompareObserver = ReturnType<
  typeof createSchemaCompareObserver
>;
export const pgSchemaCompareObserver = createSchemaCompareObserver();
