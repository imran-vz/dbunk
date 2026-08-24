/**
 * Pub/Sub tab — pattern subscription with polling drain. Phase 1.3
 * surface: pattern input, active-subscription chips, split-view
 * (channel summary + filtered message log). The discover-channels
 * sample flow is deferred.
 *
 * This file is intentionally a thin composition: the subscription
 * lifecycle, toolbar, channel list, and message log each live in
 * `./pubsub-tab/` siblings so the shell stays below fallow's
 * cognitive-complexity threshold. Sidebar geometry (width, resize,
 * collapse) is the `Panel` primitive's job.
 */

import { useMemo, useState } from "react";

import { Panel, usePanelState } from "@/components/ui/panel";

import { PubsubChannelSidebar } from "./pubsub-tab/pubsub-channel-sidebar";
import { PubsubMessageLog } from "./pubsub-tab/pubsub-message-log";
import { PubsubToolbar } from "./pubsub-tab/pubsub-toolbar";
import { usePubsubSubscription } from "./pubsub-tab/use-pubsub-subscription";

interface PubsubTabProps {
  connectionId: string;
  tabId: string;
}

export function PubsubTab({ connectionId, tabId }: PubsubTabProps) {
  const subscription = usePubsubSubscription(connectionId, tabId);
  const channelsPanel = usePanelState({
    storageKey: "dbunk.panel.pubsub-channels",
    defaultSize: 340,
    min: 280,
    max: () => Math.round(window.innerWidth * 0.5),
    snapThreshold: 140,
  });
  const [patternInput, setPatternInput] = useState("");
  const [selectedChannel, setSelectedChannel] = useState<string | null>(null);

  const filteredMessages = useMemo(
    () =>
      selectedChannel
        ? subscription.messages.filter((m) => m.channel === selectedChannel)
        : subscription.messages,
    [subscription.messages, selectedChannel],
  );

  const handleSubscribe = () => {
    const trimmed = patternInput.trim();
    if (!trimmed) return;
    setPatternInput("");
    void subscription.addPattern(trimmed);
  };

  const handleRemovePattern = (pattern: string) => {
    void subscription.removePattern(pattern);
  };

  const hasActivePatterns = subscription.activePatterns.length > 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PubsubToolbar
        connectionId={connectionId}
        patternInput={patternInput}
        onPatternInputChange={setPatternInput}
        onSubscribe={handleSubscribe}
        onSubscribeChannel={(channel) => {
          void subscription.addPattern(channel);
        }}
        hasActivePatterns={hasActivePatterns}
        paused={subscription.paused}
        onTogglePaused={subscription.togglePaused}
        onClear={subscription.clear}
        bufferedCount={subscription.messages.length}
      />
      {!hasActivePatterns ? (
        <div className="flex flex-1 items-center justify-center p-6 text-xs text-text-muted">
          Add a channel pattern above to start watching messages.
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <Panel
            side="left"
            state={channelsPanel}
            ariaLabel="Resize channels sidebar"
          >
            <PubsubChannelSidebar
              activePatterns={subscription.activePatterns}
              channelCounts={subscription.channelCounts}
              totalMessageCount={subscription.messages.length}
              selectedChannel={selectedChannel}
              onSelectChannel={setSelectedChannel}
              onRemovePattern={handleRemovePattern}
              onCollapse={channelsPanel.collapse}
            />
          </Panel>
          <main className="flex min-w-0 flex-1 flex-col">
            {subscription.error ? (
              <div className="m-4 rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
                {subscription.error}
              </div>
            ) : null}
            <div className="flex-1 overflow-auto p-3 font-mono text-xs">
              <PubsubMessageLog
                messages={filteredMessages}
                activePatterns={subscription.activePatterns}
              />
            </div>
          </main>
        </div>
      )}
    </div>
  );
}
