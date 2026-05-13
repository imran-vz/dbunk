/**
 * Owns the Pub/Sub session lifecycle for a single workspace tab:
 * starting/restarting the server-side subscription as patterns
 * change, polling the drain endpoint, capping the in-memory buffer,
 * and exposing pause/resume + clear.
 *
 * Extracted from `PubsubTab` so the component stays a thin
 * presentational shell below fallow's cognitive-complexity threshold.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  closePubsubSession,
  type DrainedMessage,
  drainPubsub,
  startPubsubSession,
} from "@/lib/redis/api";

import { DRAIN_INTERVAL_MS, MAX_BUFFER } from "./constants";

export interface PubsubChannelCount {
  channel: string;
  count: number;
}

export interface PubsubSubscriptionState {
  activePatterns: string[];
  messages: DrainedMessage[];
  channelCounts: PubsubChannelCount[];
  paused: boolean;
  error: string | null;
  addPattern: (pattern: string) => Promise<void>;
  removePattern: (pattern: string) => Promise<void>;
  togglePaused: () => void;
  clear: () => void;
}

function appendBounded(
  prev: DrainedMessage[],
  incoming: DrainedMessage[],
): DrainedMessage[] {
  const merged = [...prev, ...incoming];
  return merged.length > MAX_BUFFER
    ? merged.slice(merged.length - MAX_BUFFER)
    : merged;
}

function describeError(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function usePubsubSubscription(
  connectionId: string,
  tabId: string,
): PubsubSubscriptionState {
  const sessionId = useMemo(() => `${tabId}-${Date.now()}`, [tabId]);
  const [activePatterns, setActivePatterns] = useState<string[]>([]);
  const [messages, setMessages] = useState<DrainedMessage[]>([]);
  const [channelCounts, setChannelCounts] = useState<PubsubChannelCount[]>([]);
  const [paused, setPaused] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const sessionStartedRef = useRef(false);

  const startSession = useCallback(
    async (patterns: string[]) => {
      if (sessionStartedRef.current) {
        await closePubsubSession({ sessionId });
      }
      setError(null);
      try {
        await startPubsubSession({ connectionId, sessionId, patterns });
        sessionStartedRef.current = true;
        setPaused(false);
      } catch (err) {
        setError(describeError(err));
      }
    },
    [connectionId, sessionId],
  );

  // Tear the server-side session down when the tab unmounts so a
  // stale subscription doesn't leak server resources.
  useEffect(() => {
    return () => {
      if (sessionStartedRef.current) {
        void closePubsubSession({ sessionId });
      }
    };
  }, [sessionId]);

  useEffect(() => {
    if (paused || activePatterns.length === 0) return;
    const tick = async () => {
      try {
        const result = await drainPubsub({ sessionId });
        if (result.messages.length > 0) {
          setMessages((prev) => appendBounded(prev, result.messages));
        }
        setChannelCounts(result.channels);
      } catch (err) {
        setError(describeError(err));
      }
    };
    const interval = window.setInterval(tick, DRAIN_INTERVAL_MS);
    return () => {
      window.clearInterval(interval);
    };
  }, [paused, sessionId, activePatterns.length]);

  const addPattern = useCallback(
    async (pattern: string) => {
      const trimmed = pattern.trim();
      if (!trimmed) return;
      const next = Array.from(new Set([...activePatterns, trimmed]));
      setActivePatterns(next);
      await startSession(next);
    },
    [activePatterns, startSession],
  );

  const removePattern = useCallback(
    async (pattern: string) => {
      const next = activePatterns.filter((p) => p !== pattern);
      setActivePatterns(next);
      if (next.length === 0) {
        await closePubsubSession({ sessionId });
        sessionStartedRef.current = false;
        setPaused(true);
        return;
      }
      await startSession(next);
    },
    [activePatterns, sessionId, startSession],
  );

  const togglePaused = useCallback(() => {
    setPaused((prev) => !prev);
  }, []);

  const clear = useCallback(() => {
    setMessages([]);
    setChannelCounts([]);
  }, []);

  return {
    activePatterns,
    messages,
    channelCounts,
    paused,
    error,
    addPattern,
    removePattern,
    togglePaused,
    clear,
  };
}
