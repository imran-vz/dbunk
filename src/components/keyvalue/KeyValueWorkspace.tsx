/**
 * Top-level shell for Redis (keyvalue-class) connections — mirrors
 * the relational workspace's layout but with a keyspace browser
 * sidebar and key/cli/pubsub/server tab kinds.
 *
 * Phase 1.3: full four-tab-kind support. CLI/Server/PubSub are
 * singletons (one per connection).
 *
 * This file is intentionally a thin composition: tab orchestration,
 * sidebar chrome, tab strip, and the active-tab switch each live in
 * `./key-value-workspace/` siblings so the shell stays below
 * fallow's cognitive-complexity threshold.
 */

import { IconLayoutSidebarLeftExpand } from "@tabler/icons-react";
import { useCallback, useState } from "react";

import { NewKeyDialog } from "@/components/keyvalue/NewKeyDialog";
import { type RedisConnection, useAppStore } from "@/lib/store";
import {
  useContainerWidth,
  useResizableWidth,
} from "@/lib/use-resizable-width";

import { ActiveTabContent } from "./key-value-workspace/active-tab-content";
import {
  KEYSPACE_COMPACT_BELOW,
  KEYSPACE_DEFAULT_WIDTH,
  KEYSPACE_MAX_WIDTH,
  KEYSPACE_MIN_WIDTH,
} from "./key-value-workspace/constants";
import { KeyValueTabBar } from "./key-value-workspace/key-value-tab-bar";
import { KeyspaceSidebarPanel } from "./key-value-workspace/keyspace-sidebar-panel";
import { useKeyValueTabs } from "./key-value-workspace/use-key-value-tabs";

interface KeyValueWorkspaceProps {
  /** Narrowed at the workspace fork — see `workspace-view.tsx`'s
   *  storage-class branch. KeyValueWorkspace is only rendered for
   *  Redis connections so the prop carries the variant directly. */
  activeConnection: RedisConnection;
}

export function KeyValueWorkspace({
  activeConnection,
}: KeyValueWorkspaceProps) {
  const setActiveTabId = useAppStore((state) => state.setActiveTabId);
  const [newKeyOpen, setNewKeyOpen] = useState(false);
  const [browserRefreshTick, setBrowserRefreshTick] = useState(0);
  const [keyspaceSidebarVisible, setKeyspaceSidebarVisible] = useState(true);
  const [keyspaceOverlayOpen, setKeyspaceOverlayOpen] = useState(false);

  const { width: sidebarWidth, setWidth: setSidebarWidth } = useResizableWidth({
    storageKey: "dbunk.redis.keyspaceSidebarWidth",
    defaultWidth: KEYSPACE_DEFAULT_WIDTH,
    min: KEYSPACE_MIN_WIDTH,
    max: KEYSPACE_MAX_WIDTH,
  });

  const [containerRef, containerWidth] = useContainerWidth<HTMLDivElement>();
  const isKeyspaceCompact =
    containerWidth > 0 && containerWidth < KEYSPACE_COMPACT_BELOW;

  const {
    myTabs,
    activeTab,
    openSingleton,
    handleOpenKey,
    handleCloseTab,
    setWorkspaceTabs,
    workspaceTabs,
  } = useKeyValueTabs(activeConnection);

  // Inspector callbacks closed over `workspaceTabs` / `setWorkspaceTabs`
  // live here rather than in the hook so the hook stays narrowly
  // scoped to tab orchestration.
  const handleKeyDeleted = useCallback(
    (key: string) => {
      const deletedTab = workspaceTabs.find(
        (tab) =>
          tab.kind === "key" &&
          tab.connectionId === activeConnection.id &&
          tab.redisKey === key,
      );
      if (deletedTab) handleCloseTab(deletedTab.id);
      setBrowserRefreshTick((t) => t + 1);
    },
    [workspaceTabs, activeConnection.id, handleCloseTab],
  );

  const handleKeyRenamed = useCallback(
    (oldKey: string, newKey: string) => {
      setWorkspaceTabs((prev) =>
        prev.map((tab) =>
          tab.kind === "key" &&
          tab.connectionId === activeConnection.id &&
          tab.redisKey === oldKey
            ? { ...tab, label: newKey, redisKey: newKey }
            : tab,
        ),
      );
      setBrowserRefreshTick((t) => t + 1);
    },
    [setWorkspaceTabs, activeConnection.id],
  );

  const handleNewKeyCreated = useCallback(
    (key: string, type: string) => {
      setBrowserRefreshTick((t) => t + 1);
      handleOpenKey(key, type);
    },
    [handleOpenKey],
  );

  const showExpandStub = !isKeyspaceCompact && !keyspaceSidebarVisible;

  return (
    <div ref={containerRef} className="relative flex min-h-0 flex-1">
      {showExpandStub ? (
        <button
          type="button"
          onClick={() => setKeyspaceSidebarVisible(true)}
          aria-label="Expand keyspace sidebar"
          title="Expand keyspace sidebar"
          className="flex w-7 shrink-0 items-center justify-center border-r border-border-subtle bg-surface-window text-text-muted hover:bg-white/5 hover:text-foreground"
        >
          <IconLayoutSidebarLeftExpand className="size-4" />
        </button>
      ) : null}
      <KeyspaceSidebarPanel
        activeConnection={activeConnection}
        sidebarWidth={sidebarWidth}
        containerWidth={containerWidth}
        isKeyspaceCompact={isKeyspaceCompact}
        keyspaceSidebarVisible={keyspaceSidebarVisible}
        keyspaceOverlayOpen={keyspaceOverlayOpen}
        browserRefreshTick={browserRefreshTick}
        activeKey={activeTab?.kind === "key" ? activeTab.redisKey : undefined}
        onResize={setSidebarWidth}
        onOverlayOpenChange={setKeyspaceOverlayOpen}
        onCollapse={() => setKeyspaceSidebarVisible(false)}
        onOpenNewKey={() => setNewKeyOpen(true)}
        onOpenSingleton={openSingleton}
        onOpenKey={handleOpenKey}
      />
      <main className="flex min-w-0 flex-1 flex-col">
        <KeyValueTabBar
          myTabs={myTabs}
          activeTab={activeTab}
          onActivate={setActiveTabId}
          onClose={handleCloseTab}
        />
        <div className="flex min-h-0 flex-1">
          <ActiveTabContent
            activeConnection={activeConnection}
            activeTab={activeTab}
            onKeyDeleted={handleKeyDeleted}
            onKeyRenamed={handleKeyRenamed}
          />
        </div>
      </main>
      <NewKeyDialog
        connectionId={activeConnection.id}
        open={newKeyOpen}
        onOpenChange={setNewKeyOpen}
        onCreated={handleNewKeyCreated}
      />
    </div>
  );
}
