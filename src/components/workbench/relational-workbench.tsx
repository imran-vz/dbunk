import { useCallback, useMemo, useState } from "react";

import {
  RELATIONAL_RAIL_ITEMS,
  type WorkbenchRailId,
} from "@/components/app-shell/activity-rail";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import type { ResultsView } from "@/components/query-editor/results-view";
import type { StatusBarItem } from "@/components/status-bar";
import { TableEditorPanel } from "@/components/table-editor-panel";
import type { SubTab } from "@/components/table-editor/header";
import { ResizerHandle } from "@/components/ui/resizer-handle";
import { DatabaseNavigator } from "@/components/workbench/database-navigator";
import {
  ObjectTabRow,
  QuerySectionToggle,
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
import { QueryHistoryTab } from "@/components/workspace-overview/query-history-tab";
import { SchemaMapTab } from "@/components/workspace-overview/schema-map-tab";
import { storageClassFor } from "@/lib/engine-policy";
import { type Connection, useAppStore } from "@/lib/store";
import { useResizableWidth } from "@/lib/use-resizable-width";

const NAVIGATOR_WIDTH = 224;
const NAVIGATOR_MIN = 180;
const NAVIGATOR_MAX = 360;

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
  const [rail, setRail] = useState<WorkbenchRailId>("tables");
  const [tableSections, setTableSections] = useState<
    Record<string, TableSection>
  >({});
  const [querySections, setQuerySections] = useState<
    Record<string, ResultsView>
  >({});
  const [statusItems, setStatusItems] = useState<StatusBarItem[]>([]);

  const { width: navigatorWidth, setWidth: setNavigatorWidth } =
    useResizableWidth({
      storageKey: "dbunk.workbench.navigatorWidth",
      defaultWidth: NAVIGATOR_WIDTH,
      min: NAVIGATOR_MIN,
      max: NAVIGATOR_MAX,
    });

  const {
    activeConnectionId,
    activeTabId,
    connections,
    workspaceTabs,
    schemaExplorer,
    queryHistory,
    openSettings,
    openTableTab,
    createNewQueryTab,
    connectConnection,
    reopenHistoryEntry,
    setActiveView,
  } = useAppStore();

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
      setRail(next);
      setActiveView("workspace");
      if (
        next === "queries" &&
        workspaceTabs.every((t) => t.kind !== "query")
      ) {
        createNewQueryTab();
      }
    },
    [createNewQueryTab, setActiveView, workspaceTabs],
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

  const querySection = activeTab
    ? (querySections[activeTab.id] ?? "results")
    : "results";

  const setQuerySection = useCallback(
    (next: ResultsView) => {
      if (!activeTab) return;
      setQuerySections((current) => ({ ...current, [activeTab.id]: next }));
    },
    [activeTab],
  );

  const sectionControl = useMemo(() => {
    if (settingsView || !activeTab) return null;
    if (activeTab.kind === "table") {
      return (
        <TableSectionToggle value={tableSubTab} onChange={setTableSubTab} />
      );
    }
    if (activeTab.kind === "query") {
      return (
        <QuerySectionToggle value={querySection} onChange={setQuerySection} />
      );
    }
    return null;
  }, [
    activeTab,
    querySection,
    setQuerySection,
    setTableSubTab,
    settingsView,
    tableSubTab,
  ]);

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
          variant="workbench"
          resultsView={querySection}
          onResultsViewChange={setQuerySection}
          onStatusItemsChange={setStatusItems}
        />
      );
    }

    if (activeTab?.kind === "table") {
      return (
        <TableEditorPanel
          tab={activeTab}
          variant="workbench"
          activeSubTab={tableSectionToSubTab(tableSubTab)}
          onSubTabChange={(next) => {
            if (next === "data" || next === "schema" || next === "indexes") {
              setTableSubTab(next);
            }
          }}
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

  const leftPanel = showNavigator ? (
    <div
      className="relative flex shrink-0 flex-col"
      style={{ width: navigatorWidth }}
    >
      <DatabaseNavigator
        connectionId={activeConnection?.id ?? ""}
        schemas={schemas}
        activeTableKey={activeTableKey}
        onOpenTable={handleOpenTable}
        className="h-full w-full"
      />
      <ResizerHandle
        width={navigatorWidth}
        onResize={setNavigatorWidth}
        min={NAVIGATOR_MIN}
        max={NAVIGATOR_MAX}
        ariaLabel="Resize database navigator"
      />
    </div>
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
