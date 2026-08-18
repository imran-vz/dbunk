/* oxlint-disable anti-slop/no-runtime-typeof -- Persisted tab data is an external boundary and is validated during hydration. */
/**
 * Workspace Tabs slice — owns the Workspace Tab list, active tab ID,
 * active view, and the workspace UI flags (sidebar open/closed,
 * editor theme, selected row index).
 *
 * Exposes `closeTabsForConnection(connectionId)` as its piece of the
 * delete-connection cleanup cascade; `Connections.deleteConnection`
 * invokes it alongside the other slice cleanups (see the cascade in
 * `connections.ts`). Also exposes `retargetQueryTab(tabId, connectionId)`
 * for the editor's connection selector — flipping the tab's
 * `connectionId` and clearing the tab's stale per-tab query state via
 * `dropQueryStateForTab`.
 */

import type { StateCreator } from "zustand";

import type {
  ActiveView,
  AppStoreState,
  SettingsTab,
  WorkspaceTab,
} from "./types";

// Module-local counters survive across the slice's actions but are
// scoped to the slice file — they were globals in the monolith.
let nextTabIndex = 1;
let nextQueryIndex = 1;

export type WorkspaceTabsSlice = {
  activeView: ActiveView;
  activeTabId: string;
  workspaceTabs: WorkspaceTab[];
  isLeftSidebarOpen: boolean;
  editorTheme: string;
  selectedRowIndex: number;
  settingsTab: SettingsTab;

  setActiveView: (view: ActiveView) => void;
  setSettingsTab: (tab: SettingsTab) => void;
  /**
   * Convenience: switch to the Settings view and optionally focus a
   * specific tab. Most entry points (header gear, sidebar cog) use
   * this rather than calling `setActiveView` + `setSettingsTab`
   * separately.
   */
  openSettings: (tab?: SettingsTab) => void;
  setActiveTabId: (id: string) => void;
  setWorkspaceTabs: (
    tabs: WorkspaceTab[] | ((prev: WorkspaceTab[]) => WorkspaceTab[]),
  ) => void;
  toggleLeftSidebar: () => void;
  setEditorTheme: (theme: string) => void;
  setSelectedRowIndex: (index: number) => void;
  closeTab: (tabId: string) => void;
  openWorkspaceTab: (tab: Omit<WorkspaceTab, "id">) => void;
  openTableTab: (schemaName: string, tableName: string) => void;
  openViewTab: (schemaName: string, viewName: string) => void;
  openQueryForTable: (schemaName: string, tableName: string) => void;
  createNewQueryTab: () => void;
  createNewTableTab: () => void;
  reopenHistoryEntry: (entry: { sql: string; connectionId: string }) => void;

  /**
   * Cascade cleanup — drops every Workspace Tab whose
   * `connectionId` matches. Called by `Connections.deleteConnection`
   * (see the cascade in `connections.ts`).
   */
  closeTabsForConnection: (connectionId: string) => void;

  /**
   * Retarget a query tab to a different connection. Resets the tab's
   * grid edits / query status / preview (they belong to the old
   * connection's run) and updates `activeConnectionId` so the sidebar
   * reflects the new selection. SQL text is preserved — schema-qualified
   * references that don't exist on the new connection are the user's
   * problem to resolve.
   */
  retargetQueryTab: (tabId: string, newConnectionId: string) => void;
};

export const createWorkspaceTabsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  WorkspaceTabsSlice
> = (set, get) => ({
  activeView: "workspace",
  activeTabId: "",
  workspaceTabs: [],
  isLeftSidebarOpen: true,
  editorTheme: "vs",
  selectedRowIndex: 0,
  settingsTab: "general",

  setActiveView: (view) => set({ activeView: view }),
  setSettingsTab: (tab) => set({ settingsTab: tab }),
  openSettings: (tab) =>
    set((state) => ({
      activeView: "settings",
      settingsTab: tab ?? state.settingsTab,
    })),
  setActiveTabId: (id) => set({ activeTabId: id, activeView: "workspace" }),
  setWorkspaceTabs: (tabs) =>
    set((state) => ({
      workspaceTabs:
        typeof tabs === "function" ? tabs(state.workspaceTabs) : tabs,
    })),
  toggleLeftSidebar: () =>
    set((state) => ({ isLeftSidebarOpen: !state.isLeftSidebarOpen })),
  setEditorTheme: (theme) => set({ editorTheme: theme }),
  setSelectedRowIndex: (index) => set({ selectedRowIndex: index }),

  closeTab: (tabId) =>
    set((state) => {
      const index = state.workspaceTabs.findIndex((tab) => tab.id === tabId);
      if (index === -1) {
        return {};
      }
      const nextTabs = state.workspaceTabs.filter((tab) => tab.id !== tabId);
      let nextActiveTabId = state.activeTabId;
      if (tabId === state.activeTabId) {
        const nextTab = nextTabs[index] ?? nextTabs[index - 1];
        nextActiveTabId = nextTab?.id ?? "";
      }
      return { workspaceTabs: nextTabs, activeTabId: nextActiveTabId };
    }),

  openWorkspaceTab: (tab) => {
    const state = get();
    set({ activeView: "workspace", activeConnectionId: tab.connectionId });

    const existing = state.workspaceTabs.find(
      (item) =>
        item.kind === tab.kind &&
        item.label === tab.label &&
        item.connectionId === tab.connectionId,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }

    const id = `tab-${nextTabIndex}`;
    nextTabIndex += 1;
    set((state) => ({
      workspaceTabs: [...state.workspaceTabs, { ...tab, id }],
      activeTabId: id,
    }));
  },

  openTableTab: (schemaName, tableName) => {
    const connectionId = get().activeConnectionId;
    get().openWorkspaceTab({
      kind: "table",
      label: tableName,
      connectionId,
      schema: schemaName,
      table: tableName,
    });
    if (connectionId) {
      void get().loadTableData(connectionId, schemaName, tableName);
    }
  },

  openViewTab: (schemaName, viewName) => {
    get().openWorkspaceTab({
      kind: "query",
      label: `${viewName}.sql`,
      connectionId: get().activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.${viewName} limit 100;`,
    });
  },

  openQueryForTable: (schemaName, tableName) => {
    const state = get();
    const queryLabel = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;
    get().openWorkspaceTab({
      kind: "query",
      label: queryLabel,
      connectionId: state.activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.${tableName} limit 100;`,
    });
  },

  createNewQueryTab: () => {
    const state = get();
    const explorerSchemas =
      state.schemaExplorer[state.activeConnectionId] ?? [];
    const schemaName = explorerSchemas[0]?.name ?? "public";
    const queryLabel = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;
    get().openWorkspaceTab({
      kind: "query",
      label: queryLabel,
      connectionId: state.activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.users limit 50;`,
    });
  },

  createNewTableTab: () => {
    const state = get();
    const explorerSchemas =
      state.schemaExplorer[state.activeConnectionId] ?? [];
    const schemaName = explorerSchemas[0]?.name;
    const tableName = explorerSchemas[0]?.tables[0];
    if (!schemaName || !tableName) {
      return;
    }
    get().openTableTab(schemaName, tableName);
  },

  reopenHistoryEntry: (entry) => {
    const state = get();
    set({
      activeView: "workspace",
      activeConnectionId: entry.connectionId,
    });
    const existing = state.workspaceTabs.find(
      (item) =>
        item.kind === "query" &&
        item.connectionId === entry.connectionId &&
        (item.query ?? "") === entry.sql,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const id = `tab-${nextTabIndex}`;
    nextTabIndex += 1;
    const label = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;
    set((state) => ({
      workspaceTabs: [
        ...state.workspaceTabs,
        {
          id,
          kind: "query",
          label,
          connectionId: entry.connectionId,
          schema: "",
          query: entry.sql,
        },
      ],
      activeTabId: id,
    }));
  },

  closeTabsForConnection: (connectionId) =>
    set((state) => {
      const nextTabs = state.workspaceTabs.filter(
        (tab) => tab.connectionId !== connectionId,
      );
      const droppedActive = !nextTabs.some(
        (tab) => tab.id === state.activeTabId,
      );
      const nextActiveTab = droppedActive ? nextTabs[0] : undefined;
      return {
        workspaceTabs: nextTabs,
        activeTabId: droppedActive
          ? (nextActiveTab?.id ?? "")
          : state.activeTabId,
        activeConnectionId:
          droppedActive && nextActiveTab
            ? nextActiveTab.connectionId
            : state.activeConnectionId,
      };
    }),

  retargetQueryTab: (tabId, newConnectionId) => {
    const state = get();
    const tab = state.workspaceTabs.find((item) => item.id === tabId);
    if (!tab || tab.kind !== "query") return;
    if (tab.connectionId === newConnectionId) return;
    get().dropQueryStateForTab(tab.id, tab.label);
    set((s) => ({
      activeConnectionId: newConnectionId,
      workspaceTabs: s.workspaceTabs.map((item) =>
        item.id === tabId
          ? { ...item, connectionId: newConnectionId, lastRun: undefined }
          : item,
      ),
    }));
  },
});
