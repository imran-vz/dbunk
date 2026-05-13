/**
 * Renders the body of whichever tab is currently active in the
 * Redis workspace — key inspector, CLI, server info, or pub/sub —
 * plus the empty-state fallback.
 *
 * Extracted from `KeyValueWorkspace` so the shell stays below
 * fallow's cognitive-complexity threshold; the multi-arm conditional
 * lives here as a single focused responsibility.
 */

import { CliTab } from "@/components/keyvalue/CliTab";
import { KeyInspectorTab } from "@/components/keyvalue/KeyInspectorTab";
import { PubsubTab } from "@/components/keyvalue/PubsubTab";
import { ServerTab } from "@/components/keyvalue/ServerTab";
import type { RedisConnection, WorkspaceTab } from "@/lib/store";

interface ActiveTabContentProps {
  activeConnection: RedisConnection;
  activeTab: WorkspaceTab | undefined;
  onKeyDeleted: (key: string) => void;
  onKeyRenamed: (oldKey: string, newKey: string) => void;
}

export function ActiveTabContent({
  activeConnection,
  activeTab,
  onKeyDeleted,
  onKeyRenamed,
}: ActiveTabContentProps) {
  if (activeTab?.kind === "key" && activeTab.redisKey) {
    return (
      <KeyInspectorTab
        connectionId={activeConnection.id}
        keyName={activeTab.redisKey}
        onKeyDeleted={onKeyDeleted}
        onKeyRenamed={onKeyRenamed}
      />
    );
  }

  if (activeTab?.kind === "cli") {
    return <CliTab connectionId={activeConnection.id} />;
  }

  if (activeTab?.kind === "server") {
    return (
      <ServerTab
        connectionId={activeConnection.id}
        dbNumber={activeConnection.dbNumber ?? 0}
      />
    );
  }

  if (activeTab?.kind === "pubsub") {
    return (
      <PubsubTab connectionId={activeConnection.id} tabId={activeTab.id} />
    );
  }

  return (
    <div className="flex flex-1 items-center justify-center p-6 text-xs text-text-muted">
      Pick a tab above or open a key from the sidebar.
    </div>
  );
}
