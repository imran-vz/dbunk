/**
 * Channels-and-patterns sidebar for the Pub/Sub tab: header with a
 * collapse caret, active-pattern chips (click to unsubscribe), and
 * the per-channel message-count list with an "All channels" entry
 * that doubles as the clear-filter row.
 *
 * Extracted from `PubsubTab` to keep the parent below fallow's
 * cognitive-complexity threshold.
 */

import { Badge } from "@/components/ui/badge";

import type { PubsubChannelCount } from "./use-pubsub-subscription";

interface PubsubChannelSidebarProps {
  width: number;
  activePatterns: string[];
  channelCounts: PubsubChannelCount[];
  totalMessageCount: number;
  selectedChannel: string | null;
  onSelectChannel: (channel: string | null) => void;
  onRemovePattern: (pattern: string) => void;
  onCollapse: () => void;
}

export function PubsubChannelSidebar({
  width,
  activePatterns,
  channelCounts,
  totalMessageCount,
  selectedChannel,
  onSelectChannel,
  onRemovePattern,
  onCollapse,
}: PubsubChannelSidebarProps) {
  return (
    <aside
      style={{ width: `${width}px` }}
      className="flex shrink-0 flex-col border-r border-border-subtle bg-surface-window"
    >
      <div className="flex items-center justify-between border-b border-border-subtle px-2 py-1 text-[0.65rem] uppercase text-text-muted">
        <span className="truncate">Channels</span>
        <button
          type="button"
          onClick={onCollapse}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-label="Hide channels sidebar"
        >
          ‹
        </button>
      </div>
      <div className="flex flex-wrap gap-1 border-b border-border-subtle p-2 text-[0.65rem]">
        {activePatterns.map((p) => (
          <Badge
            key={p}
            variant="secondary"
            className="cursor-pointer hover:bg-destructive/30"
            onClick={() => onRemovePattern(p)}
          >
            {p} ×
          </Badge>
        ))}
      </div>
      <ul className="flex-1 overflow-auto text-[0.65rem]">
        <li>
          <button
            type="button"
            onClick={() => onSelectChannel(null)}
            className={`flex w-full justify-between px-2 py-1 hover:bg-white/5 ${
              selectedChannel === null
                ? "bg-primary/10 text-primary"
                : "text-text-muted"
            }`}
          >
            <span>All channels</span>
            <span>{totalMessageCount}</span>
          </button>
        </li>
        {channelCounts.map((c) => (
          <li key={c.channel}>
            <button
              type="button"
              onClick={() => onSelectChannel(c.channel)}
              className={`flex w-full justify-between gap-2 px-2 py-1 font-mono hover:bg-white/5 ${
                selectedChannel === c.channel
                  ? "bg-primary/10 text-primary"
                  : ""
              }`}
            >
              <span className="truncate">{c.channel}</span>
              <span className="shrink-0 text-text-muted">{c.count}</span>
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
