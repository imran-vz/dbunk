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

import { requestConfirm } from "@/lib/confirm";
import { resetResultMutationClientForTab } from "@/lib/result-mutation-client";
import { supportsServerTableBrowse } from "@/lib/table-browse";
import { uiGet } from "@/lib/ui-state";

import type {
  ActiveView,
  AppStoreState,
  SettingsTab,
  TabCaret,
  WorkspaceTab,
  Connection,
} from "./types";

/**
 * Validate a persisted caret candidate: present members must be finite
 * positive integers, and an anchor is only kept when both members are
 * valid. Anything else degrades to "no caret" — never to a dropped tab.
 */
export function validateTabCaret(
  caret: Partial<TabCaret> | undefined,
): TabCaret | undefined {
  if (typeof caret !== "object" || caret === null) return undefined;
  // Persisted members may hold any JSON value at runtime;
  // `Number.isInteger` rejects non-numbers before the range check.
  const isPosition = (value: number | undefined): value is number =>
    value !== undefined && Number.isInteger(value) && value >= 1;
  if (!isPosition(caret.line) || !isPosition(caret.column)) return undefined;
  const validated: TabCaret = { line: caret.line, column: caret.column };
  if (isPosition(caret.anchorLine) && isPosition(caret.anchorColumn)) {
    validated.anchorLine = caret.anchorLine;
    validated.anchorColumn = caret.anchorColumn;
  }
  return validated;
}

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
  /**
   * Monotonic counter bumped by every explicit tab open/activate (new
   * tab, palette open, Cmd+1..9, MRU switch). The workbench listens to
   * it to leave rails that don't render tabs (history, overview, …) —
   * otherwise those actions mutate the store invisibly. Session restore
   * intentionally does not bump it: the persisted rail wins at boot.
   */
  tabRevealRequest: number;
  /**
   * Monotonic counter bumped when a surface (the Open Anything palette)
   * asks the workbench to show the tables rail — the rail is local
   * state in `relational-workbench.tsx`, so this is the store-side
   * signal it subscribes to, mirroring `tabRevealRequest`. Plan 010.
   */
  railRevealRequest: number;

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
  closeTab: (tabId: string) => Promise<void>;
  openWorkspaceTab: (tab: Omit<WorkspaceTab, "id">) => void;
  openTableTab: (schemaName: string, tableName: string) => void;
  openViewTab: (schemaName: string, viewName: string) => void;
  openQueryForTable: (schemaName: string, tableName: string) => void;
  createNewQueryTab: () => void;
  createNewTableTab: () => void;
  reopenHistoryEntry: (entry: { sql: string; connectionId: string }) => void;

  /**
   * P8 session restore: rebuilds open tabs / active tab / expanded
   * tree nodes from the persisted `ui.v1.session` blob. Only tabs
   * whose connection still exists are restored; any parse failure
   * falls back to an empty session silently (corrupt-state rule).
   */
  restoreSession: () => void;

  /**
   * Record the editor caret/selection for a query tab so it survives
   * relaunch with the session blob. No-ops for missing or non-query
   * tabs. Deliberately does NOT touch `isDirty` — moving the caret is
   * not an edit. Plan 009 (caret wiring lands with Plan 010).
   */
  updateQueryCaret: (tabId: string, caret: TabCaret) => void;

  /**
   * Open Anything `reveal-schema` target: switch to the connection,
   * expand the schema node, and signal the workbench to show the
   * tables rail. Never touches the navigator's local text filter.
   */
  revealSchemaInNavigator: (connectionId: string, schema: string) => void;

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
  retargetQueryTab: (
    tabId: string,
    newConnectionId: string,
    options?: {
      confirmDiscardStagedChanges?: (
        changeCount: number,
      ) => boolean | Promise<boolean>;
      confirmProductionTarget?: (
        connection: Connection,
      ) => boolean | Promise<boolean>;
    },
  ) => Promise<boolean>;
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
  tabRevealRequest: 0,
  railRevealRequest: 0,

  setActiveView: (view) => set({ activeView: view }),
  setSettingsTab: (tab) => set({ settingsTab: tab }),
  openSettings: (tab) =>
    set((state) => ({
      activeView: "settings",
      settingsTab: tab ?? state.settingsTab,
    })),
  setActiveTabId: (id) => {
    get().markQuerySessionViewed(id);
    set((state) => ({
      activeTabId: id,
      activeView: "workspace",
      tabRevealRequest: state.tabRevealRequest + 1,
    }));
  },
  setWorkspaceTabs: (tabs) =>
    set((state) => ({
      workspaceTabs:
        typeof tabs === "function" ? tabs(state.workspaceTabs) : tabs,
    })),
  toggleLeftSidebar: () =>
    set((state) => ({ isLeftSidebarOpen: !state.isLeftSidebarOpen })),
  setEditorTheme: (theme) => set({ editorTheme: theme }),
  setSelectedRowIndex: (index) => set({ selectedRowIndex: index }),

  closeTab: async (tabId) => {
    const tabDrafts = Object.values(get().mutationDrafts).filter(
      (draft) => draft?.owner.tabId === tabId,
    );
    if (tabDrafts.some((draft) => draft?.apply.state === "applying")) {
      return;
    }
    const hasStagedChanges = tabDrafts.some(
      (draft) => draft && draft.changeOrder.length > 0,
    );
    if (
      hasStagedChanges &&
      !(await requestConfirm({
        title: "Close tab?",
        message: "Closing this tab clears its staged result changes.",
        confirmLabel: "Close tab",
        danger: true,
      }))
    ) {
      return;
    }
    await get().closeQuerySessionForTab(tabId);
    await get().closeTableBrowseForTab(tabId);
    resetResultMutationClientForTab(tabId);
    get().dropMutationDraftsForTab(tabId);
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
    });
  },

  openWorkspaceTab: (tab) => {
    const state = get();
    set((current) => ({
      activeView: "workspace",
      activeConnectionId: tab.connectionId,
      tabRevealRequest: current.tabRevealRequest + 1,
    }));

    const existing = state.workspaceTabs.find((item) => {
      if (item.kind !== tab.kind || item.connectionId !== tab.connectionId) {
        return false;
      }
      if (tab.kind === "table") {
        return item.schema === tab.schema && item.table === tab.table;
      }
      return item.label === tab.label;
    });
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
      const engine = get().connections.find(
        (connection) => connection.id === connectionId,
      )?.engine;
      if (engine && supportsServerTableBrowse(engine)) {
        void get().openTableBrowse(
          get().activeTabId,
          connectionId,
          schemaName,
          tableName,
        );
      } else {
        void get().loadTableData(connectionId, schemaName, tableName);
      }
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

  restoreSession: () => {
    const raw = uiGet("dbunk.session");
    if (!raw) return;
    const connectionIds = new Set(get().connections.map((c) => c.id));
    let tabs: WorkspaceTab[] = [];
    let activeTabId = "";
    let expandedSchemas: string[] = [];
    try {
      // SAFETY: parsed session data is validated field-by-field below.
      const parsed = JSON.parse(raw) as {
        tabs?: unknown;
        activeTabId?: unknown;
        expandedSchemas?: unknown;
      };
      if (Array.isArray(parsed.tabs)) {
        tabs = parsed.tabs.flatMap((candidate): WorkspaceTab[] => {
          if (typeof candidate !== "object" || candidate === null) return [];
          // SAFETY: every field is re-validated below before use.
          const tab = candidate as Partial<WorkspaceTab>;
          if (
            typeof tab.id !== "string" ||
            typeof tab.label !== "string" ||
            typeof tab.connectionId !== "string" ||
            typeof tab.schema !== "string" ||
            (tab.kind !== "query" && tab.kind !== "table") ||
            !connectionIds.has(tab.connectionId)
          ) {
            return [];
          }
          const restored: WorkspaceTab = {
            id: tab.id,
            kind: tab.kind,
            label: tab.label,
            connectionId: tab.connectionId,
            schema: tab.schema,
          };
          if (typeof tab.table === "string") restored.table = tab.table;
          if (typeof tab.query === "string") restored.query = tab.query;
          if (tab.pinned === true) restored.pinned = true;
          if (tab.isDirty === true) restored.isDirty = true;
          if (tab.kind === "query") {
            const caret = validateTabCaret(tab.caret);
            if (caret) restored.caret = caret;
          }
          return [restored];
        });
      }
      if (typeof parsed.activeTabId === "string") {
        activeTabId = parsed.activeTabId;
      }
      if (Array.isArray(parsed.expandedSchemas)) {
        expandedSchemas = parsed.expandedSchemas.filter(
          (value): value is string => typeof value === "string",
        );
      }
    } catch {
      return;
    }
    if (tabs.length === 0 && expandedSchemas.length === 0) return;
    // Bump the id/label counters past restored tabs so new tabs never
    // collide with restored ids.
    for (const tab of tabs) {
      const idMatch = /^tab-(\d+)$/.exec(tab.id);
      if (idMatch) {
        nextTabIndex = Math.max(nextTabIndex, Number(idMatch[1]) + 1);
      }
      const labelMatch = /^query_(\d+)\.sql$/.exec(tab.label);
      if (labelMatch) {
        nextQueryIndex = Math.max(nextQueryIndex, Number(labelMatch[1]) + 1);
      }
    }
    const active = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0];
    set((state) => {
      const next: Partial<AppStoreState> = {
        workspaceTabs: [...tabs, ...state.workspaceTabs],
        activeTabId: active?.id ?? state.activeTabId,
        expandedSchemas:
          expandedSchemas.length > 0 ? expandedSchemas : state.expandedSchemas,
      };
      if (active) next.activeConnectionId = active.connectionId;
      return next;
    });
  },

  revealSchemaInNavigator: (connectionId, schema) => {
    const schemaId = `${connectionId}:${schema}`;
    get().setExpandedSchemas((prev) =>
      prev.includes(schemaId) ? prev : [...prev, schemaId],
    );
    set((state) => ({
      activeView: "workspace",
      activeConnectionId: connectionId,
      railRevealRequest: state.railRevealRequest + 1,
    }));
  },

  updateQueryCaret: (tabId, caret) =>
    set((state) => {
      const tab = state.workspaceTabs.find((item) => item.id === tabId);
      if (!tab || tab.kind !== "query") return {};
      // Unchanged caret → no new `workspaceTabs` identity, so the
      // session persister and every tab subscriber stay quiet.
      const current = tab.caret;
      if (
        current !== undefined &&
        current.line === caret.line &&
        current.column === caret.column &&
        current.anchorLine === caret.anchorLine &&
        current.anchorColumn === caret.anchorColumn
      ) {
        return {};
      }
      return {
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId ? { ...item, caret } : item,
        ),
      };
    }),

  reopenHistoryEntry: (entry) => {
    const state = get();
    set((current) => ({
      activeView: "workspace",
      activeConnectionId: entry.connectionId,
      tabRevealRequest: current.tabRevealRequest + 1,
    }));
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

  closeTabsForConnection: (connectionId) => {
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
    });
  },

  retargetQueryTab: async (tabId, newConnectionId, options) => {
    const state = get();
    const tab = state.workspaceTabs.find((item) => item.id === tabId);
    if (!tab || tab.kind !== "query") return false;
    if (tab.connectionId === newConnectionId) return false;
    const targetConnection = state.connections.find(
      (connection) => connection.id === newConnectionId,
    );
    if (
      targetConnection?.environment === "production" &&
      !(await options?.confirmProductionTarget?.(targetConnection))
    ) {
      return false;
    }
    // Re-read after the confirm await: drafts may have changed while the
    // dialog was open.
    const stagedChangeCount = Object.values(get().mutationDrafts).reduce(
      (count, draft) =>
        draft?.owner.tabId === tabId ? count + draft.changeOrder.length : count,
      0,
    );
    if (
      stagedChangeCount > 0 &&
      !(await options?.confirmDiscardStagedChanges?.(stagedChangeCount))
    ) {
      return false;
    }
    await get().closeQuerySessionForTab(tab.id);
    resetResultMutationClientForTab(tab.id);
    get().dropMutationDraftsForTab(tab.id);
    get().dropQueryStateForTab(tab.id);
    set((s) => ({
      activeConnectionId: newConnectionId,
      workspaceTabs: s.workspaceTabs.map((item) =>
        item.id === tabId
          ? { ...item, connectionId: newConnectionId, lastRun: undefined }
          : item,
      ),
    }));
    return true;
  },
});
