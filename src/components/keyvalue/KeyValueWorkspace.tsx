/**
 * Top-level shell for Redis (keyvalue-class) connections — mirrors
 * the relational workspace's layout but with a keyspace browser
 * sidebar and key/cli/pubsub/server tab kinds.
 *
 * Phase 1.3: full four-tab-kind support. CLI/Server/PubSub are
 * singletons (one per connection).
 */

import {
  IconPlus,
  IconServer,
  IconTerminal2,
  IconWaveSine,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { CliTab } from "@/components/keyvalue/CliTab";
import { KeyInspectorTab } from "@/components/keyvalue/KeyInspectorTab";
import { KeyspaceBrowser } from "@/components/keyvalue/KeyspaceBrowser";
import { NewKeyDialog } from "@/components/keyvalue/NewKeyDialog";
import { PubsubTab } from "@/components/keyvalue/PubsubTab";
import { ServerTab } from "@/components/keyvalue/ServerTab";
import { Button } from "@/components/ui/button";
import { type Connection, useAppStore, type WorkspaceTab } from "@/lib/store";
import { cn } from "@/lib/utils";

interface KeyValueWorkspaceProps {
  activeConnection: Connection;
}

export function KeyValueWorkspace({
  activeConnection,
}: KeyValueWorkspaceProps) {
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const setWorkspaceTabs = useAppStore((state) => state.setWorkspaceTabs);
  const setActiveTabId = useAppStore((state) => state.setActiveTabId);
  const [newKeyOpen, setNewKeyOpen] = useState(false);
  const [browserRefreshTick, setBrowserRefreshTick] = useState(0);

  const myTabs = useMemo<WorkspaceTab[]>(
    () =>
      workspaceTabs.filter(
        (tab) =>
          tab.connectionId === activeConnection.id &&
          (tab.kind === "key" ||
            tab.kind === "cli" ||
            tab.kind === "pubsub" ||
            tab.kind === "server"),
      ),
    [workspaceTabs, activeConnection.id],
  );

  const activeTab = useMemo(
    () => myTabs.find((tab) => tab.id === activeTabId) ?? myTabs[0],
    [myTabs, activeTabId],
  );

  const openSingleton = (kind: "cli" | "pubsub" | "server", label: string) => {
    const existing = workspaceTabs.find(
      (tab) => tab.kind === kind && tab.connectionId === activeConnection.id,
    );
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = `redis-${kind}-${activeConnection.id}`;
    const newTab: WorkspaceTab = {
      id,
      kind,
      label,
      connectionId: activeConnection.id,
      schema: "",
    };
    setWorkspaceTabs((prev) => [...prev, newTab]);
    setActiveTabId(id);
  };

  const handleOpenKey = (key: string, _type: string) => {
    const existing = workspaceTabs.find(
      (tab) =>
        tab.kind === "key" &&
        tab.connectionId === activeConnection.id &&
        tab.redisKey === key &&
        (tab.redisDbNumber ?? 0) === (activeConnection.dbNumber ?? 0),
    );
    if (existing) {
      setActiveTabId(existing.id);
      return;
    }
    const id = `redis-key-${activeConnection.id}-${key}-${Date.now()}`;
    const newTab: WorkspaceTab = {
      id,
      kind: "key",
      label: key,
      connectionId: activeConnection.id,
      schema: "",
      redisKey: key,
      redisDbNumber: activeConnection.dbNumber ?? 0,
    };
    setWorkspaceTabs((prev) => [...prev, newTab]);
    setActiveTabId(id);
  };

  const handleCloseTab = (tabId: string) => {
    setWorkspaceTabs((prev) => prev.filter((tab) => tab.id !== tabId));
    if (activeTabId === tabId) {
      const remaining = workspaceTabs.filter((tab) => tab.id !== tabId);
      const fallback = remaining.find(
        (tab) => tab.connectionId === activeConnection.id,
      );
      setActiveTabId(fallback?.id ?? "");
    }
  };

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="flex w-64 shrink-0 flex-col border-r border-border-subtle bg-surface-window">
        <div className="flex items-center justify-between border-b border-border-subtle px-3 py-2 text-[0.65rem] uppercase tracking-wide text-text-muted">
          <span>Keyspace · DB {activeConnection.dbNumber ?? 0}</span>
          <button
            type="button"
            onClick={() => setNewKeyOpen(true)}
            className="rounded p-0.5 hover:bg-white/5 hover:text-foreground"
            aria-label="New key"
          >
            <IconPlus className="size-3" />
          </button>
        </div>
        <div className="flex shrink-0 items-center gap-1 border-b border-border-subtle px-2 py-1 text-[0.65rem]">
          <Button
            size="sm"
            variant="ghost"
            className="h-6 flex-1 px-2 text-[0.65rem]"
            onClick={() => openSingleton("server", "Server")}
          >
            <IconServer className="size-3" />
            Server
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 flex-1 px-2 text-[0.65rem]"
            onClick={() => openSingleton("cli", "CLI")}
          >
            <IconTerminal2 className="size-3" />
            CLI
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 flex-1 px-2 text-[0.65rem]"
            onClick={() => openSingleton("pubsub", "Pub/Sub")}
          >
            <IconWaveSine className="size-3" />
            Pub/Sub
          </Button>
        </div>
        <KeyspaceBrowser
          key={browserRefreshTick}
          connection={activeConnection}
          onOpenKey={handleOpenKey}
          activeKey={activeTab?.kind === "key" ? activeTab.redisKey : undefined}
        />
      </aside>
      <main className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center gap-1 overflow-auto border-b border-border-subtle bg-surface-panel/40 px-2">
          {myTabs.length === 0 ? (
            <span className="px-2 py-1.5 text-xs text-text-muted">
              Open Server, CLI, or Pub/Sub above, or click a key in the sidebar.
            </span>
          ) : (
            myTabs.map((tab) => (
              <div
                key={tab.id}
                className={cn(
                  "flex items-center gap-1 rounded-t-md px-2 py-1 text-xs",
                  activeTab?.id === tab.id
                    ? "bg-surface-panel text-foreground"
                    : "text-text-muted hover:text-foreground",
                )}
              >
                <button
                  type="button"
                  onClick={() => setActiveTabId(tab.id)}
                  className="truncate font-mono"
                >
                  {tab.label}
                </button>
                <button
                  type="button"
                  onClick={() => handleCloseTab(tab.id)}
                  aria-label={`Close ${tab.label}`}
                  className="rounded p-0.5 text-text-muted hover:text-foreground"
                >
                  ×
                </button>
              </div>
            ))
          )}
        </div>
        <div className="flex min-h-0 flex-1">
          {activeTab?.kind === "key" && activeTab.redisKey ? (
            <KeyInspectorTab
              connectionId={activeConnection.id}
              keyName={activeTab.redisKey}
              onKeyDeleted={(key) => {
                // Close the inspector tab for the deleted key.
                const deletedTab = workspaceTabs.find(
                  (tab) =>
                    tab.kind === "key" &&
                    tab.connectionId === activeConnection.id &&
                    tab.redisKey === key,
                );
                if (deletedTab) handleCloseTab(deletedTab.id);
                setBrowserRefreshTick((t) => t + 1);
              }}
              onKeyRenamed={(oldKey, newKey) => {
                // Update the inspector tab to point at the new name.
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
              }}
            />
          ) : activeTab?.kind === "cli" ? (
            <CliTab connectionId={activeConnection.id} />
          ) : activeTab?.kind === "server" ? (
            <ServerTab
              connectionId={activeConnection.id}
              dbNumber={activeConnection.dbNumber ?? 0}
            />
          ) : activeTab?.kind === "pubsub" ? (
            <PubsubTab
              connectionId={activeConnection.id}
              tabId={activeTab.id}
            />
          ) : (
            <div className="flex flex-1 items-center justify-center p-6 text-xs text-text-muted">
              Pick a tab above or open a key from the sidebar.
            </div>
          )}
        </div>
      </main>
      <NewKeyDialog
        connectionId={activeConnection.id}
        open={newKeyOpen}
        onOpenChange={setNewKeyOpen}
        onCreated={(key, type) => {
          setBrowserRefreshTick((t) => t + 1);
          handleOpenKey(key, type);
        }}
      />
    </div>
  );
}
