import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  RELATIONAL_RAIL_ITEMS,
  type WorkbenchRailId,
} from "@/components/app-shell/activity-rail";
import { ConnectionsView } from "@/components/connections-view";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import type { StatusBarItem } from "@/components/status-bar";
import { TableEditorPanel } from "@/components/table-editor-panel";
import type { SubTab } from "@/components/table-editor/header";
import { Panel, useLayoutPressure, usePanelState } from "@/components/ui/panel";
import { DatabaseNavigator } from "@/components/workbench/database-navigator";
import {
  ObjectTabRow,
  type TableSection,
  TableSectionToggle,
} from "@/components/workbench/object-tab-row";
import { firstRelationalConnection } from "@/components/workbench/workbench-policy";
import { WorkbenchShell } from "@/components/workbench/workbench-shell";
import { AdminTab } from "@/components/workspace-overview/admin-tab";
import {
  DisconnectedConnectionCard,
  NoConnectionCard,
} from "@/components/workspace-overview/disconnected-card";
import { OverviewRailView } from "@/components/workspace-overview/overview-rail-view";
import { QueryHistoryTab } from "@/components/workspace-overview/query-history-tab";
import { SchemaMapTab } from "@/components/workspace-overview/schema-map-tab";
import { storageClassFor } from "@/lib/engine-policy";
import { useShortcutHandler } from "@/lib/shortcuts";
import { type Connection, useAppStore } from "@/lib/store";
import { uiGet, uiSet } from "@/lib/ui-state";

/** Navigator metrics per DESIGN-SYSTEM §3.3. */
const NAVIGATOR_DEFAULT = 260;
const NAVIGATOR_MIN = 180;
const NAVIGATOR_SNAP = 90;
const NAVIGATOR_MAX = () => Math.round(window.innerWidth * 0.5);
/** Activity rail (40) + 1px borders — chrome that never yields. */
const FIXED_CHROME_WIDTH = 42;

interface RelationalWorkbenchProps {
  isClient: boolean;
  isWindowFullscreen: boolean;
  onPointerDown: React.PointerEventHandler<HTMLElement>;
  onDoubleClick: React.MouseEventHandler<HTMLElement>;
  settingsView?: React.ReactNode;
}

function isConnected(connection: Connection): boolean {
  return connection.status === "Connected" || connection.status === "Read only";
}

function tableSectionToSubTab(section: TableSection): SubTab {
  return section;
}

export function RelationalWorkbench({
  isClient,
  isWindowFullscreen,
  onPointerDown,
  onDoubleClick,
  settingsView,
}: RelationalWorkbenchProps) {
  // P8: last rail view persists across relaunch.
  const [rail, setRail] = useState<WorkbenchRailId>(() => {
    const stored = uiGet("dbunk.workbench.rail");
    const match = RELATIONAL_RAIL_ITEMS.find((item) => item.id === stored);
    return match ? match.id : "tables";
  });
  useEffect(() => {
    uiSet("dbunk.workbench.rail", rail);
  }, [rail]);
  const [tableSections, setTableSections] = useState<
    Record<string, TableSection>
  >({});
  const [statusItems, setStatusItems] = useState<StatusBarItem[]>([]);

  const navigatorRef = useRef<HTMLDivElement>(null);
  const navigatorPanel = usePanelState({
    storageKey: "dbunk.workbench.navigator",
    defaultSize: NAVIGATOR_DEFAULT,
    min: NAVIGATOR_MIN,
    max: NAVIGATOR_MAX,
    snapThreshold: NAVIGATOR_SNAP,
  });

  useLayoutPressure({
    fixedWidth: FIXED_CHROME_WIDTH,
    navigatorState: navigatorPanel,
  });

  // Cmd+B toggles the navigator (§3.2 restore paths).
  const toggleNavigator = navigatorPanel.toggle;
  useShortcutHandler("toggle-navigator", toggleNavigator);
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "b") {
        event.preventDefault();
        toggleNavigator();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleNavigator]);

  const activeConnectionId = useAppStore((state) => state.activeConnectionId);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const connections = useAppStore((state) => state.connections);
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const schemaExplorer = useAppStore((state) => state.schemaExplorer);
  const queryHistory = useAppStore((state) => state.queryHistory);
  const openSettings = useAppStore((state) => state.openSettings);
  const openTableTab = useAppStore((state) => state.openTableTab);
  const createNewQueryTab = useAppStore((state) => state.createNewQueryTab);
  const connectConnection = useAppStore((state) => state.connectConnection);
  const reopenHistoryEntry = useAppStore((state) => state.reopenHistoryEntry);
  const setActiveView = useAppStore((state) => state.setActiveView);
  const tabRevealRequest = useAppStore((state) => state.tabRevealRequest);

  // Tab shortcuts and palette actions (Cmd+T, Cmd+1..9, open table…)
  // mutate the store, but rail screens render before the tab panels —
  // from History/Overview/Admin the opened tab would be invisible.
  // Follow explicit reveals back to a tab-rendering rail.
  const lastRevealRef = useRef(tabRevealRequest);
  useEffect(() => {
    if (tabRevealRequest === lastRevealRef.current) return;
    lastRevealRef.current = tabRevealRequest;
    setRail((current) => {
      if (current === "tables" || current === "queries") return current;
      const state = useAppStore.getState();
      const revealed = state.workspaceTabs.find(
        (tab) => tab.id === state.activeTabId,
      );
      if (!revealed) return current;
      return revealed.kind === "query" ? "queries" : "tables";
    });
  }, [tabRevealRequest]);

  const activeConnection = useMemo(() => {
    const selected = connections.find(
      (connection) => connection.id === activeConnectionId,
    );
    if (selected && storageClassFor(selected.engine) === "relational") {
      return selected;
    }
    return firstRelationalConnection(connections);
  }, [activeConnectionId, connections]);

  const activeTab = useMemo(
    () => workspaceTabs.find((tab) => tab.id === activeTabId),
    [activeTabId, workspaceTabs],
  );

  const schemas = activeConnection
    ? (schemaExplorer[activeConnection.id] ?? [])
    : [];

  const activeTableKey =
    activeTab?.kind === "table" && activeTab.table
      ? `${activeTab.schema}.${activeTab.table}`
      : null;

  const showNavigator =
    !settingsView && (rail === "tables" || rail === "queries");

  const handleOpenSettings = useCallback(() => {
    openSettings();
    setActiveView("settings");
  }, [openSettings, setActiveView]);

  const handleRailChange = useCallback(
    (next: WorkbenchRailId) => {
      // Clicking the active rail item toggles its navigator (the rail
      // doubles as the sidebar's restore affordance, §3.1).
      if (next === rail && (next === "tables" || next === "queries")) {
        navigatorPanel.toggle();
      } else if (next === "tables" || next === "queries") {
        navigatorPanel.expand();
      }
      setRail(next);
      setActiveView("workspace");
      if (
        next === "queries" &&
        workspaceTabs.every((t) => t.kind !== "query")
      ) {
        createNewQueryTab();
      }
    },
    [createNewQueryTab, navigatorPanel, rail, setActiveView, workspaceTabs],
  );

  const handleOpenTable = useCallback(
    (schema: string, table: string) => {
      setRail("tables");
      setActiveView("workspace");
      openTableTab(schema, table);
    },
    [openTableTab, setActiveView],
  );

  const tableSubTab: TableSection = activeTab
    ? (tableSections[activeTab.id] ?? "data")
    : "data";

  const setTableSubTab = useCallback(
    (next: TableSection) => {
      if (!activeTab) return;
      setTableSections((current) => ({ ...current, [activeTab.id]: next }));
    },
    [activeTab],
  );

  // Query tabs carry their Results/Explain toggle inside the results
  // pane itself (§5.2); only table tabs keep a section control here.
  const sectionControl = useMemo(() => {
    if (settingsView || !activeTab) return null;
    if (activeTab.kind === "table") {
      return (
        <TableSectionToggle value={tableSubTab} onChange={setTableSubTab} />
      );
    }
    return null;
  }, [activeTab, setTableSubTab, settingsView, tableSubTab]);

  const renderMainPane = () => {
    if (settingsView) {
      return (
        <div className="min-h-0 flex-1 overflow-hidden">{settingsView}</div>
      );
    }

    if (!activeConnection) {
      return <NoConnectionCard />;
    }

    if (!isConnected(activeConnection) && schemas.length === 0) {
      return (
        <DisconnectedConnectionCard
          connection={activeConnection}
          onNewQuery={createNewQueryTab}
          onConnect={() => {
            void connectConnection(activeConnection.id);
          }}
        />
      );
    }

    if (rail === "connections") {
      return (
        <div className="min-h-0 flex-1 overflow-hidden">
          <ConnectionsView variant="rail" />
        </div>
      );
    }

    if (rail === "overview") {
      return (
        <OverviewRailView
          activeConnection={activeConnection}
          schemas={schemas}
          isConnected={isConnected(activeConnection)}
          onOpenTable={handleOpenTable}
          onReopenQuery={reopenHistoryEntry}
        />
      );
    }

    if (rail === "schema-map") {
      return (
        <SchemaMapTab
          activeConnection={activeConnection}
          schemas={schemas}
          isClient={isClient}
        />
      );
    }

    if (rail === "history") {
      return (
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <QueryHistoryTab
            activeConnection={activeConnection}
            queryHistory={queryHistory}
            onReopenEntry={reopenHistoryEntry}
          />
        </div>
      );
    }

    if (rail === "admin") {
      return (
        <div className="min-h-0 flex-1 overflow-auto p-4">
          <AdminTab connection={activeConnection} />
        </div>
      );
    }

    if (activeTab?.kind === "query") {
      return (
        <QueryEditorPanel
          tab={activeTab}
          isClient={isClient}
          onStatusItemsChange={setStatusItems}
        />
      );
    }

    if (activeTab?.kind === "table") {
      return (
        <TableEditorPanel
          tab={activeTab}
          activeSubTab={tableSectionToSubTab(tableSubTab)}
          onSubTabChange={setTableSubTab}
          onStatusItemsChange={setStatusItems}
        />
      );
    }

    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-sm text-text-muted">
        Select a table from the navigator or open a query tab.
      </div>
    );
  };

  const showObjectTabs =
    !settingsView &&
    (rail === "tables" || rail === "queries") &&
    workspaceTabs.length > 0;

  const autoFitNavigator = useCallback(() => {
    // Double-click auto-fit: size to the widest visible tree label
    // (§3.4), measured as the tree's scroll width.
    const el = navigatorRef.current;
    if (!el) return;
    const content = el.querySelector('[data-slot="navigator-tree"]') ?? el;
    navigatorPanel.setSize(content.scrollWidth + 16);
  }, [navigatorPanel]);

  const leftPanel = showNavigator ? (
    <Panel
      side="left"
      state={navigatorPanel}
      ariaLabel="Resize database navigator"
      onAutoFit={autoFitNavigator}
    >
      <div ref={navigatorRef} className="flex min-h-0 flex-1 flex-col">
        <DatabaseNavigator
          connectionId={activeConnection?.id ?? ""}
          schemas={schemas}
          activeTableKey={activeTableKey}
          onOpenTable={handleOpenTable}
          className="h-full w-full"
        />
      </div>
    </Panel>
  ) : null;

  const toolbar = showObjectTabs ? (
    <ObjectTabRow sectionControl={sectionControl} />
  ) : null;

  return (
    <WorkbenchShell
      activeConnection={activeConnection}
      isWindowFullscreen={isWindowFullscreen}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      railItems={RELATIONAL_RAIL_ITEMS}
      activeRail={settingsView ? "tables" : rail}
      onRailChange={handleRailChange}
      onOpenSettings={handleOpenSettings}
      statusItems={statusItems}
      leftPanel={leftPanel}
      toolbar={toolbar}
    >
      {renderMainPane()}
    </WorkbenchShell>
  );
}
