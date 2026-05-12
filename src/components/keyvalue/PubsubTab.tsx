/**
 * Pub/Sub tab — pattern subscription with polling drain. Phase 1.3
 * surface: pattern input, active-subscription chips, split-view
 * (channel summary + filtered message log). The discover-channels
 * sample flow is deferred.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  closePubsubSession,
  type DrainedMessage,
  drainPubsub,
  formatValueOneLine,
  startPubsubSession,
} from "@/lib/redis/api";

interface PubsubTabProps {
  connectionId: string;
  tabId: string;
}

const DRAIN_INTERVAL_MS = 750;
const MAX_BUFFER = 10_000;

export function PubsubTab({ connectionId, tabId }: PubsubTabProps) {
  const sessionId = useMemo(() => `${tabId}-${Date.now()}`, [tabId]);
  const [patternInput, setPatternInput] = useState("");
  const [activePatterns, setActivePatterns] = useState<string[]>([]);
  const [messages, setMessages] = useState<DrainedMessage[]>([]);
  const [channelCounts, setChannelCounts] = useState<
    Array<{ channel: string; count: number }>
  >([]);
  const [selectedChannel, setSelectedChannel] = useState<string | null>(null);
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
        setError(err instanceof Error ? err.message : String(err));
      }
    },
    [connectionId, sessionId],
  );

  useEffect(() => {
    return () => {
      if (sessionStartedRef.current) {
        void closePubsubSession({ sessionId });
      }
    };
  }, [sessionId]);

  useEffect(() => {
    if (paused || activePatterns.length === 0) return;
    const interval = window.setInterval(async () => {
      try {
        const result = await drainPubsub({ sessionId });
        if (result.messages.length > 0) {
          setMessages((prev) => {
            const merged = [...prev, ...result.messages];
            return merged.length > MAX_BUFFER
              ? merged.slice(merged.length - MAX_BUFFER)
              : merged;
          });
        }
        setChannelCounts(result.channels);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }, DRAIN_INTERVAL_MS);
    return () => {
      window.clearInterval(interval);
    };
  }, [paused, sessionId, activePatterns.length]);

  const addPattern = async () => {
    const trimmed = patternInput.trim();
    if (!trimmed) return;
    const next = Array.from(new Set([...activePatterns, trimmed]));
    setPatternInput("");
    setActivePatterns(next);
    await startSession(next);
  };

  const removePattern = async (pattern: string) => {
    const next = activePatterns.filter((p) => p !== pattern);
    setActivePatterns(next);
    if (next.length === 0) {
      await closePubsubSession({ sessionId });
      sessionStartedRef.current = false;
      setPaused(true);
      return;
    }
    await startSession(next);
  };

  const filteredMessages = selectedChannel
    ? messages.filter((m) => m.channel === selectedChannel)
    : messages;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-panel/60 px-4 py-2 text-xs">
        <Input
          value={patternInput}
          onChange={(event) => setPatternInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              void addPattern();
            }
          }}
          placeholder="Channel pattern (e.g. notifications.*)"
          className="h-7 max-w-xs text-xs"
        />
        <Button
          size="sm"
          className="h-7 px-2 text-xs"
          disabled={!patternInput.trim()}
          onClick={() => {
            void addPattern();
          }}
        >
          Subscribe
        </Button>
        {activePatterns.length > 0 ? (
          <Button
            size="sm"
            variant={paused ? "default" : "outline"}
            className="h-7 px-2 text-xs"
            onClick={() => setPaused((prev) => !prev)}
          >
            {paused ? "Resume" : "Pause"}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-xs"
          onClick={() => {
            setMessages([]);
            setChannelCounts([]);
          }}
        >
          Clear
        </Button>
        <span className="ml-auto text-text-muted">
          {messages.length.toLocaleString()} messages buffered
        </span>
      </header>
      {activePatterns.length === 0 ? (
        <div className="flex flex-1 items-center justify-center p-6 text-xs text-text-muted">
          Add a channel pattern above to start watching messages.
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <aside className="flex w-64 shrink-0 flex-col border-r border-border-subtle bg-surface-window">
            <div className="flex flex-wrap gap-1 border-b border-border-subtle p-2 text-[0.65rem]">
              {activePatterns.map((p) => (
                <Badge
                  key={p}
                  variant="secondary"
                  className="cursor-pointer hover:bg-destructive/30"
                  onClick={() => {
                    void removePattern(p);
                  }}
                >
                  {p} ×
                </Badge>
              ))}
            </div>
            <div className="border-b border-border-subtle px-2 py-1 text-[0.65rem] uppercase text-text-muted">
              Channels seen
            </div>
            <ul className="flex-1 overflow-auto text-[0.65rem]">
              <li>
                <button
                  type="button"
                  onClick={() => setSelectedChannel(null)}
                  className={`flex w-full justify-between px-2 py-1 hover:bg-white/5 ${
                    selectedChannel === null
                      ? "bg-primary/10 text-primary"
                      : "text-text-muted"
                  }`}
                >
                  <span>All channels</span>
                  <span>{messages.length}</span>
                </button>
              </li>
              {channelCounts.map((c) => (
                <li key={c.channel}>
                  <button
                    type="button"
                    onClick={() => setSelectedChannel(c.channel)}
                    className={`flex w-full justify-between px-2 py-1 font-mono hover:bg-white/5 ${
                      selectedChannel === c.channel
                        ? "bg-primary/10 text-primary"
                        : ""
                    }`}
                  >
                    <span className="truncate">{c.channel}</span>
                    <span className="text-text-muted">{c.count}</span>
                  </button>
                </li>
              ))}
            </ul>
          </aside>
          <main className="flex min-w-0 flex-1 flex-col">
            {error ? (
              <div className="m-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                {error}
              </div>
            ) : null}
            <div className="flex-1 overflow-auto p-3 font-mono text-xs">
              {filteredMessages.length === 0 ? (
                <p className="text-text-muted">
                  Waiting for messages on {activePatterns.join(", ")}…
                </p>
              ) : (
                <ul className="space-y-0.5">
                  {filteredMessages.slice(-500).map((msg, idx) => (
                    <li
                      key={`${msg.receivedAtMs}-${msg.channel}-${idx}`}
                      className="border-b border-border-subtle py-1"
                    >
                      <span className="text-text-muted">
                        [
                        {new Date(msg.receivedAtMs).toISOString().slice(11, 23)}
                        ]
                      </span>{" "}
                      <span className="text-primary">{msg.channel}</span>{" "}
                      <span className="break-all">
                        {formatValueOneLine(msg.payload)}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </main>
        </div>
      )}
    </div>
  );
}
