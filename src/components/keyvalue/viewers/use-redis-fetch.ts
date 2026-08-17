import { useEffect, useRef } from "react";

interface UseRedisFetchOptions<T> {
  fetch: () => Promise<T>;
  /**
   * Runs synchronously when `cacheKey` changes, before the fetch
   * starts. Reset loading/error/data shape here.
   */
  onStart: () => void;
  /** Receives the result on success (skipped if cancelled). */
  onSuccess: (result: T) => void;
  /** Receives the normalised error message (skipped if cancelled). */
  onError: (message: string) => void;
  /** Runs when the fetch settles (skipped if cancelled). */
  onSettled: () => void;
  /**
   * String identity of the request. The effect re-fires when this
   * value changes — equivalent to the deps array of a `useEffect`,
   * but encoded as a single primitive so the linter can verify it.
   * Compose from the inputs that affect the fetch (e.g. `${id}|${k}`).
   */
  cacheKey: string;
}

/**
 * Shared async-fetch primitive for the Redis value viewers. Handles
 * cancellation (via a request-sequence counter that survives across
 * `cacheKey`-driven retriggers — including the manual reload pattern
 * in the Hash viewer) and `Error → string` normalisation. The caller
 * owns all UI state and writes through the `on*` callbacks.
 */
export function useRedisFetch<T>({
  fetch,
  onStart,
  onSuccess,
  onError,
  onSettled,
  cacheKey,
}: UseRedisFetchOptions<T>): void {
  const requestSeq = useRef(0);

  /* oxlint-disable react-hooks/exhaustive-deps -- cacheKey is the change trigger; callbacks are caller-controlled */
  useEffect(() => {
    let cancelled = false;
    const seq = ++requestSeq.current;
    onStart();
    fetch()
      .then((result) => {
        if (cancelled || requestSeq.current !== seq) return;
        onSuccess(result);
      })
      .catch((err) => {
        if (cancelled || requestSeq.current !== seq) return;
        onError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled && requestSeq.current === seq) onSettled();
      });
    return () => {
      cancelled = true;
    };
  }, [cacheKey]);
  /* oxlint-enable react-hooks/exhaustive-deps */
}
