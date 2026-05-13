/**
 * Left-edge keyspace sidebar for the Redis workspace.
 *
 * Wraps `ResponsiveEdgePanel` and renders the compact + wide header,
 * the singleton-tab launchers (Server / CLI / Pub/Sub), and the
 * `KeyspaceBrowser`. Extracted from `KeyValueWorkspace` to keep
 * the shell below fallow's cognitive-complexity threshold.
 */

import {
  IconLayoutSidebarLeftCollapse,
  IconPin,
  IconPinnedOff,
  IconPlus,
  IconServer,
  IconTerminal2,
  IconWaveSine,
} from "@tabler/icons-react";

import { KeyspaceBrowser } from "@/components/keyvalue/KeyspaceBrowser";
import { Button } from "@/components/ui/button";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
import type { RedisConnection } from "@/lib/store";

import {
  KEYSPACE_COMPACT_BELOW,
  KEYSPACE_MAX_WIDTH,
  KEYSPACE_MIN_WIDTH,
  PROTECTED_WORKSPACE_WIDTH,
} from "./constants";

type SingletonKind = "cli" | "pubsub" | "server";

interface KeyspaceSidebarPanelProps {
  activeConnection: RedisConnection;
  sidebarWidth: number;
  containerWidth: number;
  isKeyspaceCompact: boolean;
  keyspaceSidebarVisible: boolean;
  keyspaceOverlayOpen: boolean;
  browserRefreshTick: number;
  activeKey: string | undefined;
  onResize: (width: number) => void;
  onOverlayOpenChange: (open: boolean) => void;
  onCollapse: () => void;
  onOpenNewKey: () => void;
  onOpenSingleton: (kind: SingletonKind, label: string) => void;
  onOpenKey: (key: string, type: string) => void;
}

export function KeyspaceSidebarPanel({
  activeConnection,
  sidebarWidth,
  containerWidth,
  isKeyspaceCompact,
  keyspaceSidebarVisible,
  keyspaceOverlayOpen,
  browserRefreshTick,
  activeKey,
  onResize,
  onOverlayOpenChange,
  onCollapse,
  onOpenNewKey,
  onOpenSingleton,
  onOpenKey,
}: KeyspaceSidebarPanelProps) {
  const dbNumber = activeConnection.dbNumber ?? 0;

  return (
    <ResponsiveEdgePanel
      side="left"
      storageKey="dbunk.sidebar.redisKeyspace"
      title="Keyspace"
      width={sidebarWidth}
      containerWidth={containerWidth}
      compactBelow={KEYSPACE_COMPACT_BELOW}
      protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
      wideVisible={keyspaceSidebarVisible}
      open={keyspaceOverlayOpen}
      onOpenChange={onOverlayOpenChange}
      resizer={{
        onResize,
        min: KEYSPACE_MIN_WIDTH,
        max: KEYSPACE_MAX_WIDTH,
        ariaLabel: "Resize keyspace sidebar",
      }}
      renderCompactHeader={({ pinned, onTogglePinned, onClose }) => (
        <CompactHeader
          dbNumber={dbNumber}
          pinned={pinned}
          onTogglePinned={onTogglePinned}
          onClose={onClose}
          onOpenNewKey={onOpenNewKey}
        />
      )}
    >
      <div className="flex h-full min-h-0 flex-col">
        {!isKeyspaceCompact ? (
          <WideHeader
            dbNumber={dbNumber}
            onCollapse={onCollapse}
            onOpenNewKey={onOpenNewKey}
          />
        ) : null}
        <SingletonLauncherRow onOpenSingleton={onOpenSingleton} />
        <KeyspaceBrowser
          key={browserRefreshTick}
          connection={activeConnection}
          onOpenKey={onOpenKey}
          activeKey={activeKey}
        />
      </div>
    </ResponsiveEdgePanel>
  );
}

interface CompactHeaderProps {
  dbNumber: number;
  pinned: boolean;
  onTogglePinned: () => void;
  onClose: () => void;
  onOpenNewKey: () => void;
}

function CompactHeader({
  dbNumber,
  pinned,
  onTogglePinned,
  onClose,
  onOpenNewKey,
}: CompactHeaderProps) {
  const pinLabel = pinned ? "Unpin keyspace sidebar" : "Pin keyspace sidebar";
  return (
    <div className="flex items-center justify-between gap-2 border-b border-border-subtle px-3 py-2 text-[0.65rem] uppercase tracking-wide text-text-muted">
      <span className="truncate">Keyspace · DB {dbNumber}</span>
      <div className="flex shrink-0 items-center">
        <button
          type="button"
          onClick={onOpenNewKey}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-label="New key"
          title="New key"
        >
          <IconPlus className="size-3" />
        </button>
        <button
          type="button"
          onClick={onTogglePinned}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-pressed={pinned}
          aria-label={pinLabel}
          title={pinLabel}
        >
          {pinned ? (
            <IconPinnedOff className="size-3" />
          ) : (
            <IconPin className="size-3" />
          )}
        </button>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-label="Collapse keyspace sidebar"
          title="Collapse keyspace sidebar"
        >
          <IconLayoutSidebarLeftCollapse className="size-3" />
        </button>
      </div>
    </div>
  );
}

interface WideHeaderProps {
  dbNumber: number;
  onCollapse: () => void;
  onOpenNewKey: () => void;
}

function WideHeader({ dbNumber, onCollapse, onOpenNewKey }: WideHeaderProps) {
  return (
    <div className="flex items-center justify-between gap-2 border-b border-border-subtle px-3 py-2 text-[0.65rem] uppercase tracking-wide text-text-muted">
      <span className="truncate">Keyspace · DB {dbNumber}</span>
      <div className="flex shrink-0 items-center">
        <button
          type="button"
          onClick={onOpenNewKey}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-label="New key"
          title="New key"
        >
          <IconPlus className="size-3" />
        </button>
        <button
          type="button"
          onClick={onCollapse}
          className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
          aria-label="Collapse keyspace sidebar"
          title="Collapse keyspace sidebar"
        >
          <IconLayoutSidebarLeftCollapse className="size-3" />
        </button>
      </div>
    </div>
  );
}

interface SingletonLauncherRowProps {
  onOpenSingleton: (kind: SingletonKind, label: string) => void;
}

function SingletonLauncherRow({ onOpenSingleton }: SingletonLauncherRowProps) {
  return (
    <div className="flex shrink-0 items-center gap-1 border-b border-border-subtle px-2 py-1 text-[0.65rem]">
      <Button
        size="sm"
        variant="ghost"
        className="h-6 min-w-0 flex-1 px-1.5 text-[0.65rem]"
        onClick={() => onOpenSingleton("server", "Server")}
      >
        <IconServer className="size-3" />
        <span className="truncate">Server</span>
      </Button>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 min-w-0 flex-1 px-1.5 text-[0.65rem]"
        onClick={() => onOpenSingleton("cli", "CLI")}
      >
        <IconTerminal2 className="size-3" />
        <span className="truncate">CLI</span>
      </Button>
      <Button
        size="sm"
        variant="ghost"
        className="h-6 min-w-0 flex-1 px-1.5 text-[0.65rem]"
        onClick={() => onOpenSingleton("pubsub", "Pub/Sub")}
      >
        <IconWaveSine className="size-3" />
        <span className="truncate">Pub/Sub</span>
      </Button>
    </div>
  );
}
