/**
 * Top-level shell for Redis (keyvalue-class) connections — the
 * "Command Center" layout.
 *
 * No nested sidebar. Instead:
 * - A top command bar with connection info, section switcher
 *   (Keys / CLI / Server), and a key search.
 * - The "keys" section renders the keyspace browser as a full-width
 *   grid/table — clicking a key opens the inspector in a right
 *   slide-over panel.
 * - CLI and Server are full-pane views.
 *
 * This replaces the previous sidebar+tabs layout that created a
 * sidebar-within-sidebar visual problem.
 */

import {
  IconBroadcast,
  IconDatabase,
  IconKey,
  IconPlus,
  IconServer,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useState } from "react";

import { CliTab } from "@/components/keyvalue/CliTab";
import { KeyInspectorTab } from "@/components/keyvalue/KeyInspectorTab";
import { KeyspaceBrowser } from "@/components/keyvalue/KeyspaceBrowser";
import { NewKeyDialog } from "@/components/keyvalue/NewKeyDialog";
import { PubsubTab } from "@/components/keyvalue/PubsubTab";
import { ServerTab } from "@/components/keyvalue/ServerTab";
import { type RedisConnection, useAppStore } from "@/lib/store";

type Section = "keys" | "cli" | "server" | "pubsub";

interface KeyValueWorkspaceProps {
  activeConnection: RedisConnection;
  variant?: "default" | "workbench";
  activeSection?: Section;
}

export function KeyValueWorkspace({
  activeConnection,
  variant = "default",
  activeSection: controlledSection,
}: KeyValueWorkspaceProps) {
  const [internalSection, setInternalSection] = useState<Section>("keys");
  const activeSection = controlledSection ?? internalSection;
  const setActiveSection = controlledSection ? () => {} : setInternalSection;
  const isWorkbench = variant === "workbench";
  const [selectedKey, setSelectedKey] = useState<{
    name: string;
    type: string;
  } | null>(null);
  const [newKeyOpen, setNewKeyOpen] = useState(false);
  const [browserRefreshTick, setBrowserRefreshTick] = useState(0);

  // Stable tab ID for CLI session — one per connection mount
  const [cliTabId] = useState(
    () =>
      `cli-${activeConnection.id}-${globalThis.crypto?.randomUUID?.() ?? Date.now()}`,
  );
  const [pubsubTabId] = useState(
    () =>
      `pubsub-${activeConnection.id}-${globalThis.crypto?.randomUUID?.() ?? Date.now()}`,
  );

  const capabilities = useAppStore(
    (state) => state.redisCapabilitiesByConnection[activeConnection.id],
  );

  const handleOpenKey = useCallback((key: string, type: string) => {
    setSelectedKey({ name: key, type });
  }, []);

  const handleKeyDeleted = useCallback(
    (key: string) => {
      if (selectedKey?.name === key) {
        setSelectedKey(null);
      }
      setBrowserRefreshTick((t) => t + 1);
    },
    [selectedKey],
  );

  const handleKeyRenamed = useCallback(
    (oldKey: string, newKey: string) => {
      if (selectedKey?.name === oldKey) {
        setSelectedKey({ name: newKey, type: selectedKey.type });
      }
      setBrowserRefreshTick((t) => t + 1);
    },
    [selectedKey],
  );

  const handleNewKeyCreated = useCallback(
    (key: string, type: string) => {
      setBrowserRefreshTick((t) => t + 1);
      handleOpenKey(key, type);
    },
    [handleOpenKey],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!isWorkbench ? (
        <div className="flex items-center gap-3 border-b border-border-subtle bg-surface-window px-4 py-2">
          <div className="flex items-center gap-2">
            <IconDatabase className="size-4 text-accent" />
            <span className="text-sm font-medium text-foreground">
              {activeConnection.name}
            </span>
            <span className="rounded bg-surface-panel-elevated px-1.5 py-0.5 text-2xs text-text-muted">
              {activeConnection.host}:{activeConnection.port}/db
              {activeConnection.dbNumber}
            </span>
            {capabilities?.role && (
              <span className="rounded bg-surface-panel-elevated px-1.5 py-0.5 text-2xs text-text-muted">
                {capabilities.role}
              </span>
            )}
          </div>

          <div className="flex items-center gap-1 rounded-lg border border-border-subtle bg-surface-panel px-1 py-0.5">
            {(
              [
                { id: "keys", icon: IconKey, label: "Keys" },
                { id: "cli", icon: IconTerminal2, label: "CLI" },
                { id: "pubsub", icon: IconBroadcast, label: "Pub/Sub" },
                { id: "server", icon: IconServer, label: "Server" },
              ] as const
            ).map((section) => (
              <button
                key={section.id}
                type="button"
                onClick={() => setActiveSection(section.id)}
                className={`flex items-center gap-1 rounded-md px-3 py-1 text-xs font-medium transition-colors ${
                  activeSection === section.id
                    ? "bg-accent/15 text-accent"
                    : "text-text-muted hover:text-foreground"
                }`}
              >
                <section.icon className="size-3" />
                {section.label}
              </button>
            ))}
          </div>

          <div className="ml-auto flex items-center gap-3">
            {capabilities?.dbSize != null && (
              <span className="text-2xs text-text-muted">
                {capabilities.dbSize.toLocaleString()} keys
              </span>
            )}
            {capabilities?.serverVersion && (
              <span className="text-2xs text-text-muted">
                v{capabilities.serverVersion}
              </span>
            )}
            <button
              type="button"
              onClick={() => setNewKeyOpen(true)}
              className="flex items-center gap-1 rounded-md border border-border-subtle px-2 py-1 text-xs text-text-muted transition-colors hover:border-accent hover:text-accent"
            >
              <IconPlus className="size-3" />
              New Key
            </button>
          </div>
        </div>
      ) : (
        <div className="flex items-center gap-3 border-b border-border-subtle bg-surface-window px-4 py-2">
          <div className="ml-auto flex items-center gap-3">
            {capabilities?.dbSize != null && (
              <span className="text-2xs text-text-muted">
                {capabilities.dbSize.toLocaleString()} keys
              </span>
            )}
            {capabilities?.serverVersion && (
              <span className="text-2xs text-text-muted">
                v{capabilities.serverVersion}
              </span>
            )}
            {capabilities?.role && (
              <span className="rounded bg-surface-panel-elevated px-1.5 py-0.5 text-2xs text-text-muted">
                {capabilities.role}
              </span>
            )}
            {activeSection === "keys" ? (
              <button
                type="button"
                onClick={() => setNewKeyOpen(true)}
                className="flex items-center gap-1 rounded-md border border-border-subtle px-2 py-1 text-xs text-text-muted transition-colors hover:border-accent hover:text-accent"
              >
                <IconPlus className="size-3" />
                New Key
              </button>
            ) : null}
          </div>
        </div>
      )}

      {/* Main content */}
      <div className="flex min-h-0 flex-1">
        {activeSection === "keys" && (
          <div className="flex min-h-0 flex-1">
            {/* Keyspace browser as main content — no sidebar */}
            <div className="flex min-w-0 flex-1 flex-col">
              <KeyspaceBrowser
                key={browserRefreshTick}
                connection={activeConnection}
                onOpenKey={handleOpenKey}
                activeKey={selectedKey?.name}
              />
            </div>

            {/* Right slide-over inspector */}
            {selectedKey && (
              <div className="flex h-full w-110 shrink-0 flex-col border-l border-border-subtle bg-surface-panel">
                <div className="flex items-center justify-between border-b border-border-subtle px-4 py-2">
                  <div className="flex items-center gap-2 min-w-0">
                    <span className="rounded bg-surface-panel-elevated px-1.5 py-0.5 text-2xs font-medium text-text-muted">
                      {selectedKey.type}
                    </span>
                    <span className="truncate font-mono text-xs text-foreground">
                      {selectedKey.name}
                    </span>
                  </div>
                  <button
                    type="button"
                    onClick={() => setSelectedKey(null)}
                    className="rounded p-1 text-text-muted hover:bg-white/5 hover:text-foreground"
                    aria-label="Close inspector"
                  >
                    <IconX className="size-4" />
                  </button>
                </div>
                <div className="flex min-h-0 flex-1">
                  <KeyInspectorTab
                    connectionId={activeConnection.id}
                    keyName={selectedKey.name}
                    onKeyDeleted={handleKeyDeleted}
                    onKeyRenamed={handleKeyRenamed}
                  />
                </div>
              </div>
            )}
          </div>
        )}

        {activeSection === "cli" && (
          <div className="flex min-h-0 flex-1 flex-col">
            <CliTab connectionId={activeConnection.id} tabId={cliTabId} />
          </div>
        )}

        {activeSection === "server" && (
          <div className="flex min-h-0 flex-1 flex-col overflow-auto">
            <ServerTab
              connectionId={activeConnection.id}
              dbNumber={activeConnection.dbNumber ?? 0}
            />
          </div>
        )}

        {activeSection === "pubsub" && (
          <div className="flex min-h-0 flex-1 flex-col">
            <PubsubTab connectionId={activeConnection.id} tabId={pubsubTabId} />
          </div>
        )}
      </div>

      <NewKeyDialog
        connectionId={activeConnection.id}
        open={newKeyOpen}
        onOpenChange={setNewKeyOpen}
        onCreated={handleNewKeyCreated}
      />
    </div>
  );
}
