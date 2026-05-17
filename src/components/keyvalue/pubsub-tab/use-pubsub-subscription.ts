/**
 * Owns the Pub/Sub session lifecycle for a single workspace tab:
 * starting/restarting the server-side subscription as patterns
 * change, listening for push-delivered messages via Tauri's event
 * channel, capping the in-memory buffer, and exposing pause/resume +
 * clear.
 *
 * Push model (replaces the older 750ms polling drain): the backend
 * emits each Pub/Sub message as a `pubsub-message` event tagged with
 * the session ID; this hook filters by session ID. Right after
 * starting a session we still issue one `drain` to catch up on
 * anything the backend buffered between worker-spawn and our
 * listener attaching.
 *
 * Extracted from `PubsubTab` so the component stays a thin
 * presentational shell below fallow's cognitive-complexity threshold.
 */

import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  closePubsubSession,
  type DrainedMessage,
  drainPubsub,
  startPubsubSession,
} from "@/lib/redis/api";
import { isTauri } from "@/lib/tauri";

import { MAX_BUFFER } from "./constants";

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

type PubsubEventPayload = {
  sessionId: string;
  message: DrainedMessage;
};

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

function recountChannels(messages: DrainedMessage[]): PubsubChannelCount[] {
  const counts = new Map<string, number>();
  for (const message of messages) {
    counts.set(message.channel, (counts.get(message.channel) ?? 0) + 1);
  }
  return Array.from(counts.entries())
    .map(([channel, count]) => ({ channel, count }))
    .sort((a, b) => b.count - a.count);
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
        // Catch-up drain: backend buffers between worker-spawn and
        // listener-attach so messages emitted in that ~ms gap are
        // recoverable. Once drained, the listener takes over.
        try {
          const result = await drainPubsub({ sessionId });
          if (result.messages.length > 0) {
            setMessages((prev) => {
              const next = appendBounded(prev, result.messages);
              setChannelCounts(recountChannels(next));
              return next;
            });
          }
        } catch (err) {
          // Catch-up drain failures are non-fatal — the listener
          // alone is enough for the live stream.
          console.warn("pubsub catch-up drain failed", err);
        }
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

  // Single push-message listener — filters by sessionId so multiple
  // pub/sub tabs can coexist on the same Tauri event channel.
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    listen<PubsubEventPayload>("pubsub-message", (event) => {
      if (cancelled) return;
      if (event.payload.sessionId !== sessionId) return;
      if (paused) return;
      setMessages((prev) => {
        const next = appendBounded(prev, [event.payload.message]);
        setChannelCounts(recountChannels(next));
        return next;
      });
    })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        if (!cancelled) setError(describeError(err));
      });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [sessionId, paused]);

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
