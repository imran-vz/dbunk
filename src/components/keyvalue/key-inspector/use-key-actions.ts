import { useCallback, useState } from "react";

import { delRedisKeys, renameRedisKey, setRedisExpire } from "@/lib/redis/api";

export interface KeyActionsParams {
  connectionId: string;
  keyName: string;
  onKeyDeleted?: (key: string) => void;
  onKeyRenamed?: (oldKey: string, newKey: string) => void;
  onAfterExpire?: () => void;
}

export interface KeyActions {
  actionError: string | null;
  clearActionError: () => void;
  deleteKey: () => Promise<boolean>;
  renameKey: (newName: string) => Promise<boolean>;
  setExpire: (rawSeconds: string) => Promise<boolean>;
}

function parseExpireSeconds(raw: string): number | null {
  if (!raw.trim()) return null;
  const seconds = Number(raw);
  return Number.isFinite(seconds) ? seconds : null;
}

/**
 * Wraps the delete / rename / expire mutations. Each returns `true` on success
 * so the caller can close its dialog without rethreading try/catch. Errors land
 * on `actionError` and surface in the inline banner.
 */
export function useKeyActions({
  connectionId,
  keyName,
  onKeyDeleted,
  onKeyRenamed,
  onAfterExpire,
}: KeyActionsParams): KeyActions {
  const [actionError, setActionError] = useState<string | null>(null);

  const clearActionError = useCallback(() => setActionError(null), []);

  const deleteKey = useCallback(async () => {
    setActionError(null);
    try {
      await delRedisKeys({ connectionId, keys: [keyName] });
      onKeyDeleted?.(keyName);
      return true;
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err));
      return false;
    }
  }, [connectionId, keyName, onKeyDeleted]);

  const renameKey = useCallback(
    async (newName: string) => {
      setActionError(null);
      try {
        await renameRedisKey({ connectionId, from: keyName, to: newName });
        onKeyRenamed?.(keyName, newName);
        return true;
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
        return false;
      }
    },
    [connectionId, keyName, onKeyRenamed],
  );

  const setExpire = useCallback(
    async (rawSeconds: string) => {
      setActionError(null);
      try {
        await setRedisExpire({
          connectionId,
          key: keyName,
          ttlSeconds: parseExpireSeconds(rawSeconds),
        });
        onAfterExpire?.();
        return true;
      } catch (err) {
        setActionError(err instanceof Error ? err.message : String(err));
        return false;
      }
    },
    [connectionId, keyName, onAfterExpire],
  );

  return {
    actionError,
    clearActionError,
    deleteKey,
    renameKey,
    setExpire,
  };
}
