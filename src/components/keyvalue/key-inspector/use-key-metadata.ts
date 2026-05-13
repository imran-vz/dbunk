import { useCallback, useEffect, useState } from "react";

import { fetchKeyMetadata, type KeyMetadata } from "@/lib/redis/api";

export interface KeyMetadataState {
  metadata: KeyMetadata | null;
  error: string | null;
  refresh: () => void;
}

/**
 * Owns the load-by-(connectionId, keyName) + manual-refresh lifecycle for a
 * single inspected key. The `refreshTick` state is intentionally not part of
 * the public API — callers use `refresh()` instead.
 */
export function useKeyMetadata(
  connectionId: string,
  keyName: string,
): KeyMetadataState {
  const [metadata, setMetadata] = useState<KeyMetadata | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshTick is an intentional re-trigger
  useEffect(() => {
    let cancelled = false;
    setError(null);
    fetchKeyMetadata({ connectionId, key: keyName })
      .then((result) => {
        if (!cancelled) setMetadata(result);
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, refreshTick]);

  const refresh = useCallback(() => {
    setRefreshTick((t) => t + 1);
  }, []);

  return { metadata, error, refresh };
}
