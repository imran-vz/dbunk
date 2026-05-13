/**
 * Top-of-main tab strip for the Redis workspace.
 *
 * Renders an empty-state hint when no tabs are open, otherwise lists
 * each of the connection's open tabs with a close button. Extracted
 * from `KeyValueWorkspace` to keep that shell below fallow's
 * cognitive-complexity threshold.
 */

import type { WorkspaceTab } from "@/lib/store";
import { cn } from "@/lib/utils";

interface KeyValueTabBarProps {
  myTabs: WorkspaceTab[];
  activeTab: WorkspaceTab | undefined;
  onActivate: (tabId: string) => void;
  onClose: (tabId: string) => void;
}

export function KeyValueTabBar({
  myTabs,
  activeTab,
  onActivate,
  onClose,
}: KeyValueTabBarProps) {
  if (myTabs.length === 0) {
    return (
      <div className="flex shrink-0 items-center gap-1 overflow-auto border-b border-border-subtle bg-surface-panel/40 px-2">
        <span className="px-2 py-1.5 text-xs text-text-muted">
          Open Server, CLI, or Pub/Sub above, or click a key in the sidebar.
        </span>
      </div>
    );
  }

  return (
    <div className="flex shrink-0 items-center gap-1 overflow-auto border-b border-border-subtle bg-surface-panel/40 px-2">
      {myTabs.map((tab) => (
        <KeyValueTab
          key={tab.id}
          tab={tab}
          isActive={activeTab?.id === tab.id}
          onActivate={onActivate}
          onClose={onClose}
        />
      ))}
    </div>
  );
}

interface KeyValueTabProps {
  tab: WorkspaceTab;
  isActive: boolean;
  onActivate: (tabId: string) => void;
  onClose: (tabId: string) => void;
}

function KeyValueTab({ tab, isActive, onActivate, onClose }: KeyValueTabProps) {
  return (
    <div
      className={cn(
        "flex items-center gap-1 rounded-t-md px-2 py-1 text-xs",
        isActive
          ? "bg-surface-panel text-foreground"
          : "text-text-muted hover:text-foreground",
      )}
    >
      <button
        type="button"
        onClick={() => onActivate(tab.id)}
        className="truncate font-mono"
      >
        {tab.label}
      </button>
      <button
        type="button"
        onClick={() => onClose(tab.id)}
        aria-label={`Close ${tab.label}`}
        className="rounded p-0.5 text-text-muted hover:text-foreground"
      >
        ×
      </button>
    </div>
  );
}
