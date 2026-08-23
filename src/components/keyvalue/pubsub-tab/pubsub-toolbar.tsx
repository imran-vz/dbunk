/**
 * Top-of-tab toolbar for the Pub/Sub workspace: pattern input,
 * subscribe button, channel discovery dropdown, pause/resume toggle
 * (visible only when a subscription is live), clear button, and the
 * buffered-message count.
 *
 * Extracted from `PubsubTab` to keep the parent component below
 * fallow's cognitive-complexity threshold.
 */

import { useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  type DiscoveredChannel,
  discoverPubsubChannels,
  publishPubsubMessage,
} from "@/lib/redis/api";

interface PubsubToolbarProps {
  connectionId: string;
  patternInput: string;
  onPatternInputChange: (value: string) => void;
  onSubscribe: () => void;
  onSubscribeChannel: (channel: string) => void;
  hasActivePatterns: boolean;
  paused: boolean;
  onTogglePaused: () => void;
  onClear: () => void;
  bufferedCount: number;
}

export function PubsubToolbar({
  connectionId,
  patternInput,
  onPatternInputChange,
  onSubscribe,
  onSubscribeChannel,
  hasActivePatterns,
  paused,
  onTogglePaused,
  onClear,
  bufferedCount,
}: PubsubToolbarProps) {
  const trimmedInput = patternInput.trim();
  const [discoverOpen, setDiscoverOpen] = useState(false);
  const [channels, setChannels] = useState<DiscoveredChannel[]>([]);
  const [discoverLoading, setDiscoverLoading] = useState(false);
  const [discoverError, setDiscoverError] = useState<string | null>(null);
  const [publishOpen, setPublishOpen] = useState(false);
  const [publishChannel, setPublishChannel] = useState("");
  const [publishMessage, setPublishMessage] = useState("");
  const [publishing, setPublishing] = useState(false);

  const handlePublish = async () => {
    const channel = publishChannel.trim();
    if (!channel) return;
    setPublishing(true);
    try {
      const result = await publishPubsubMessage({
        connectionId,
        channel,
        message: publishMessage,
      });
      toast.success(
        result.receivers === 1
          ? `Published to ${channel} · 1 subscriber received it`
          : `Published to ${channel} · ${result.receivers} subscribers received it`,
      );
      setPublishMessage("");
    } catch (err) {
      toast.error(
        err instanceof Error ? `Publish failed: ${err.message}` : String(err),
      );
    } finally {
      setPublishing(false);
    }
  };

  const handleDiscover = async () => {
    setDiscoverLoading(true);
    setDiscoverError(null);
    setDiscoverOpen(true);
    try {
      const result = await discoverPubsubChannels({
        connectionId,
        pattern: trimmedInput || "*",
      });
      setChannels(result.channels);
    } catch (err) {
      setDiscoverError(err instanceof Error ? err.message : String(err));
    } finally {
      setDiscoverLoading(false);
    }
  };

  return (
    <header className="relative flex flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-panel/60 px-4 py-2 text-xs">
      <Input
        value={patternInput}
        onChange={(event) => onPatternInputChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onSubscribe();
          }
        }}
        placeholder="Channel pattern (e.g. notifications.*)"
        className="h-7 max-w-xs text-xs"
      />
      <Button
        size="sm"
        className="h-7 px-2 text-xs"
        disabled={!trimmedInput}
        onClick={onSubscribe}
      >
        Subscribe
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="h-7 px-2 text-xs"
        onClick={() => {
          void handleDiscover();
        }}
        disabled={discoverLoading}
      >
        {discoverLoading ? "Discovering…" : "Discover"}
      </Button>
      {hasActivePatterns ? (
        <Button
          size="sm"
          variant={paused ? "default" : "outline"}
          className="h-7 px-2 text-xs"
          onClick={onTogglePaused}
        >
          {paused ? "Resume" : "Pause"}
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="outline"
        className="h-7 px-2 text-xs"
        onClick={() => setPublishOpen((value) => !value)}
        aria-expanded={publishOpen}
      >
        {publishOpen ? "Close publish" : "Publish"}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        className="h-7 px-2 text-xs"
        onClick={onClear}
      >
        Clear
      </Button>
      <span className="ml-auto text-text-muted">
        {bufferedCount.toLocaleString()} messages buffered
      </span>
      {publishOpen ? (
        <div className="flex w-full flex-wrap items-center gap-2 border-t border-border-subtle pt-2 text-xs">
          <Input
            value={publishChannel}
            onChange={(event) => setPublishChannel(event.target.value)}
            placeholder="channel"
            className="h-7 max-w-xs text-xs"
            aria-label="Publish channel"
          />
          <Input
            value={publishMessage}
            onChange={(event) => setPublishMessage(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void handlePublish();
              }
            }}
            placeholder="message"
            className="h-7 flex-1 text-xs"
            aria-label="Publish message"
          />
          <Button
            size="sm"
            className="h-7 px-2 text-xs"
            disabled={!publishChannel.trim() || publishing}
            onClick={() => {
              void handlePublish();
            }}
          >
            {publishing ? "Publishing…" : "Send"}
          </Button>
        </div>
      ) : null}
      {discoverOpen ? (
        <div className="absolute left-4 top-full z-10 mt-1 w-80 rounded-md border border-border-subtle bg-surface-window shadow-lg">
          <div className="flex items-center justify-between border-b border-border-subtle px-2 py-1 text-2xs text-text-muted">
            <span>PUBSUB CHANNELS · {channels.length} found</span>
            <button
              type="button"
              className="hover:text-foreground"
              onClick={() => setDiscoverOpen(false)}
            >
              ×
            </button>
          </div>
          <div className="max-h-64 overflow-auto">
            {discoverError ? (
              <div className="px-3 py-2 text-2xs text-danger">
                {discoverError}
              </div>
            ) : channels.length === 0 && !discoverLoading ? (
              <div className="px-3 py-2 text-2xs text-text-muted">
                No active channels right now. Channels become visible once they
                have a subscriber.
              </div>
            ) : (
              <ul>
                {channels.map((entry) => (
                  <li key={entry.channel}>
                    <button
                      type="button"
                      className="flex w-full items-center justify-between gap-2 px-3 py-1 text-left font-mono text-2xs hover:bg-white/5"
                      onClick={() => {
                        onSubscribeChannel(entry.channel);
                        setDiscoverOpen(false);
                      }}
                    >
                      <span className="truncate">{entry.channel}</span>
                      <span className="text-text-muted">
                        {entry.subscribers} sub
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      ) : null}
    </header>
  );
}
