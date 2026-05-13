/**
 * Tab orchestration for the Redis (keyvalue) workspace shell.
 *
 * Filters the global workspace-tab list down to this connection's
 * tabs, derives the active tab, and exposes openers/closers used by
 * the toolbar, keyspace browser, and inspector callbacks.
 *
 * Extracted from `KeyValueWorkspace` to keep that shell below
 * fallow's cognitive-complexity threshold.
 */

import { useCallback, useMemo } from "react";

import {
  type RedisConnection,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

/** Singleton tab kinds — one per connection. */
type SingletonKind = "cli" | "pubsub" | "server";

type SetWorkspaceTabs = (
  tabs: WorkspaceTab[] | ((prev: WorkspaceTab[]) => WorkspaceTab[]),
) => void;

export interface UseKeyValueTabsResult {
  /** Workspace tabs that belong to the active Redis connection. */
  myTabs: WorkspaceTab[];
  /** Currently-active tab (falls back to first owned tab). */
  activeTab: WorkspaceTab | undefined;
  /** Open (or focus) a singleton tab — CLI / Server / Pub/Sub. */
  openSingleton: (kind: SingletonKind, label: string) => void;
  /** Open (or focus) a key inspector tab for `key`. */
  handleOpenKey: (key: string, type: string) => void;
  /** Close a tab; picks a sensible fallback if it was active. */
  handleCloseTab: (tabId: string) => void;
  /** Bulk setter for callers that need to rename/remove key tabs. */
  setWorkspaceTabs: SetWorkspaceTabs;
  /** Raw workspace-tab list (needed by inspector key-deletion callback). */
  workspaceTabs: WorkspaceTab[];
}

export function useKeyValueTabs(
  activeConnection: RedisConnection,
): UseKeyValueTabsResult {
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const setWorkspaceTabs = useAppStore((state) => state.setWorkspaceTabs);
  const setActiveTabId = useAppStore((state) => state.setActiveTabId);

  const connectionId = activeConnection.id;
  const dbNumber = activeConnection.dbNumber ?? 0;

  const myTabs = useMemo<WorkspaceTab[]>(
    () =>
      workspaceTabs.filter(
        (tab) =>
          tab.connectionId === connectionId &&
          (tab.kind === "key" ||
            tab.kind === "cli" ||
            tab.kind === "pubsub" ||
            tab.kind === "server"),
      ),
    [workspaceTabs, connectionId],
  );

  const activeTab = useMemo(
    () => myTabs.find((tab) => tab.id === activeTabId) ?? myTabs[0],
    [myTabs, activeTabId],
  );

  const openSingleton = useCallback(
    (kind: SingletonKind, label: string) => {
      const existing = workspaceTabs.find(
        (tab) => tab.kind === kind && tab.connectionId === connectionId,
      );
      if (existing) {
        setActiveTabId(existing.id);
        return;
      }
      const id = `redis-${kind}-${connectionId}`;
      const newTab: WorkspaceTab = {
        id,
        kind,
        label,
        connectionId,
        schema: "",
      };
      setWorkspaceTabs((prev) => [...prev, newTab]);
      setActiveTabId(id);
    },
    [workspaceTabs, connectionId, setActiveTabId, setWorkspaceTabs],
  );

  const handleOpenKey = useCallback(
    (key: string, _type: string) => {
      const existing = workspaceTabs.find(
        (tab) =>
          tab.kind === "key" &&
          tab.connectionId === connectionId &&
          tab.redisKey === key &&
          (tab.redisDbNumber ?? 0) === dbNumber,
      );
      if (existing) {
        setActiveTabId(existing.id);
        return;
      }
      const id = `redis-key-${connectionId}-${key}-${Date.now()}`;
      const newTab: WorkspaceTab = {
        id,
        kind: "key",
        label: key,
        connectionId,
        schema: "",
        redisKey: key,
        redisDbNumber: dbNumber,
      };
      setWorkspaceTabs((prev) => [...prev, newTab]);
      setActiveTabId(id);
    },
    [workspaceTabs, connectionId, dbNumber, setActiveTabId, setWorkspaceTabs],
  );

  const handleCloseTab = useCallback(
    (tabId: string) => {
      setWorkspaceTabs((prev) => prev.filter((tab) => tab.id !== tabId));
      if (activeTabId === tabId) {
        const remaining = workspaceTabs.filter((tab) => tab.id !== tabId);
        const fallback = remaining.find(
          (tab) => tab.connectionId === connectionId,
        );
        setActiveTabId(fallback?.id ?? "");
      }
    },
    [
      activeTabId,
      workspaceTabs,
      connectionId,
      setActiveTabId,
      setWorkspaceTabs,
    ],
  );

  return {
    myTabs,
    activeTab,
    openSingleton,
    handleOpenKey,
    handleCloseTab,
    setWorkspaceTabs,
    workspaceTabs,
  };
}
