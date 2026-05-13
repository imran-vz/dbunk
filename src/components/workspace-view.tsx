import { useMemo, useState } from "react";

import { KeyValueWorkspace } from "@/components/keyvalue/KeyValueWorkspace";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import { StatusBar } from "@/components/status-bar";
import { TableEditorPanel } from "@/components/table-editor-panel";
import { ConnectionDetailsCard } from "@/components/workspace-overview/connection-details-card";
import { DatabaseStatsCard } from "@/components/workspace-overview/database-stats-card";
import { DetailsTab } from "@/components/workspace-overview/details-tab";
import {
  DisconnectedConnectionCard,
  NoConnectionCard,
} from "@/components/workspace-overview/disconnected-card";
import { FavoriteTablesCard } from "@/components/workspace-overview/favorite-tables-card";
import { HealthBanner } from "@/components/workspace-overview/health-banner";
import { OverviewHeader } from "@/components/workspace-overview/overview-header";
import { PostgresOnlyPanel } from "@/components/workspace-overview/placeholder-panel";
import { QueryHistoryTab } from "@/components/workspace-overview/query-history-tab";
import { RecentQueriesCard } from "@/components/workspace-overview/recent-queries-card";
import { SchemasTab } from "@/components/workspace-overview/schemas-tab";
import { SettingsTab } from "@/components/workspace-overview/settings-tab";
import { TablesTab } from "@/components/workspace-overview/tables-tab";
import { useDatabaseOverview } from "@/components/workspace-overview/use-database-overview";
import { WorkspaceTabs } from "@/components/workspace-tabs";
import { storageClassFor } from "@/lib/engine-policy";
import {
  type Connection,
  type DatabaseOverviewStats,
  type DatabaseOverviewStatsStatus,
  type OverviewTabId,
  type QueryHistoryEntry,
  type RelationInfo,
  type RelationStatsStatus,
  type SchemaExplorer,
  type ServerDetails,
  type ServerDetailsStatus,
  useAppStore,
} from "@/lib/store";

interface WorkspaceViewProps {
  isClient: boolean;
}

export function WorkspaceView({ isClient }: WorkspaceViewProps) {
  const {
    activeConnectionId,
    activeTabId,
    connections,
    connectionOverviewTab,
    databaseOverviewStats,
    databaseOverviewStatsStatus,
    queryHistory,
    schemaExplorer,
    workspaceTabs,
    createNewQueryTab,
    connectConnection,
    disconnectConnection,
    loadDatabaseOverviewStats,
    loadRelationStats,
    loadServerDetails,
    openTableTab,
    relationStats,
    relationStatsStatus,
    reopenHistoryEntry,
    serverDetails,
    serverDetailsStatus,
    setConnectionOverviewTab,
  } = useAppStore();

  const activeConnection = useMemo(
    () =>
      connections.find((connection) => connection.id === activeConnectionId) ??
      connections[0],
    [activeConnectionId, connections],
  );

  const activeTab = useMemo(
    () => workspaceTabs.find((tab) => tab.id === activeTabId),
    [activeTabId, workspaceTabs],
  );

  // Storage-class fork (ADR-0008): keyvalue engines (Redis) render
  // their own workspace shell with sidebar + key/cli/pubsub/server
  // tab kinds. Forking here — above the relational tab-kind dispatch
  // — keeps `TableEditorPanel` / `QueryEditorPanel` from ever seeing
  // a Redis connection or a non-relational tab kind.
  if (
    activeConnection &&
    storageClassFor(activeConnection.engine) === "keyvalue" &&
    activeConnection.engine === "Redis"
  ) {
    // Engine-tag narrow alongside the storage-class check so TypeScript
    // can hand `KeyValueWorkspace` a `RedisConnection`. Today Redis is
    // the only keyvalue engine; the additional discriminator-check
    // disappears the moment another keyvalue engine joins the union.
    return <KeyValueWorkspace activeConnection={activeConnection} />;
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkspaceTabs />
      <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
        {activeTab && isRelationalTab(activeTab.kind) ? (
          activeTab.kind === "query" ? (
            <QueryEditorPanel tab={activeTab} isClient={isClient} />
          ) : (
            <TableEditorPanel tab={activeTab} />
          )
        ) : (
          <WorkspaceDatabaseOverview
            activeConnection={activeConnection}
            schemas={
              activeConnection
                ? (schemaExplorer[activeConnection.id] ?? [])
                : []
            }
            stats={
              activeConnection
                ? databaseOverviewStats[activeConnection.id]
                : undefined
            }
            statsStatus={
              activeConnection
                ? databaseOverviewStatsStatus[activeConnection.id]
                : undefined
            }
            queryHistory={queryHistory}
            overviewTab={
              activeConnection
                ? (connectionOverviewTab[activeConnection.id] ?? "overview")
                : "overview"
            }
            relationStats={
              activeConnection ? relationStats[activeConnection.id] : undefined
            }
            relationStatsStatus={
              activeConnection
                ? relationStatsStatus[activeConnection.id]
                : undefined
            }
            serverDetails={
              activeConnection ? serverDetails[activeConnection.id] : undefined
            }
            serverDetailsStatus={
              activeConnection
                ? serverDetailsStatus[activeConnection.id]
                : undefined
            }
            onSetOverviewTab={setConnectionOverviewTab}
            onLoadStats={loadDatabaseOverviewStats}
            onLoadRelationStats={loadRelationStats}
            onLoadServerDetails={loadServerDetails}
            onOpenTable={openTableTab}
            onNewQuery={createNewQueryTab}
            onConnectConnection={connectConnection}
            onDisconnectConnection={disconnectConnection}
            onReopenHistoryEntry={reopenHistoryEntry}
          />
        )}
      </div>
    </div>
  );
}

/** Narrow a workspace tab kind to the relational set (`table`/
 * `query`). Returns `false` for the keyvalue kinds (`key`/`cli`/
 * `pubsub`/`server`) so the relational dispatch can't accidentally
 * mount a Redis tab. */
function isRelationalTab(kind: string): kind is "table" | "query" {
  return kind === "table" || kind === "query";
}

type WorkspaceDatabaseOverviewProps = {
  activeConnection?: Connection;
  schemas: SchemaExplorer[];
  stats?: DatabaseOverviewStats;
  statsStatus?: DatabaseOverviewStatsStatus;
  queryHistory: QueryHistoryEntry[];
  overviewTab: OverviewTabId;
  relationStats: RelationInfo[] | undefined;
  relationStatsStatus: RelationStatsStatus | undefined;
  serverDetails: ServerDetails | undefined;
  serverDetailsStatus: ServerDetailsStatus | undefined;
  onSetOverviewTab: (connectionId: string, tab: OverviewTabId) => void;
  onLoadStats: (connectionId: string) => Promise<void>;
  onLoadRelationStats: (connectionId: string) => Promise<void>;
  onLoadServerDetails: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onNewQuery: () => void;
  onConnectConnection: (connectionId: string) => Promise<void>;
  onDisconnectConnection: (connectionId: string) => void;
  onReopenHistoryEntry: (entry: QueryHistoryEntry) => void;
};

function WorkspaceDatabaseOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  queryHistory,
  overviewTab,
  relationStats,
  relationStatsStatus,
  serverDetails,
  serverDetailsStatus,
  onSetOverviewTab,
  onLoadStats,
  onLoadRelationStats,
  onLoadServerDetails,
  onOpenTable,
  onNewQuery,
  onConnectConnection,
  onDisconnectConnection,
  onReopenHistoryEntry,
}: WorkspaceDatabaseOverviewProps) {
  if (!activeConnection) {
    return <NoConnectionCard />;
  }

  // Storage-class fork (ADR-0008). Keyvalue engines (Redis) get their
  // own workspace shell.
  if (
    storageClassFor(activeConnection.engine) === "keyvalue" &&
    activeConnection.engine === "Redis"
  ) {
    return <KeyValueWorkspace activeConnection={activeConnection} />;
  }

  const isConnected =
    activeConnection.status === "Connected" ||
    activeConnection.status === "Read only";

  if (!isConnected && schemas.length === 0) {
    return (
      <DisconnectedConnectionCard
        connection={activeConnection}
        onNewQuery={onNewQuery}
        onConnect={() => {
          void onConnectConnection(activeConnection.id);
        }}
      />
    );
  }

  return (
    <ConnectedOverview
      activeConnection={activeConnection}
      schemas={schemas}
      stats={stats}
      statsStatus={statsStatus}
      queryHistory={queryHistory}
      isConnected={isConnected}
      overviewTab={overviewTab}
      relationStats={relationStats}
      relationStatsStatus={relationStatsStatus}
      serverDetails={serverDetails}
      serverDetailsStatus={serverDetailsStatus}
      onSetOverviewTab={onSetOverviewTab}
      onLoadStats={onLoadStats}
      onLoadRelationStats={onLoadRelationStats}
      onLoadServerDetails={onLoadServerDetails}
      onOpenTable={onOpenTable}
      onDisconnectConnection={onDisconnectConnection}
      onReopenHistoryEntry={onReopenHistoryEntry}
    />
  );
}

type ConnectedOverviewProps = {
  activeConnection: Connection;
  schemas: SchemaExplorer[];
  stats: DatabaseOverviewStats | undefined;
  statsStatus: DatabaseOverviewStatsStatus | undefined;
  queryHistory: QueryHistoryEntry[];
  isConnected: boolean;
  overviewTab: OverviewTabId;
  relationStats: RelationInfo[] | undefined;
  relationStatsStatus: RelationStatsStatus | undefined;
  serverDetails: ServerDetails | undefined;
  serverDetailsStatus: ServerDetailsStatus | undefined;
  onSetOverviewTab: (connectionId: string, tab: OverviewTabId) => void;
  onLoadStats: (connectionId: string) => Promise<void>;
  onLoadRelationStats: (connectionId: string) => Promise<void>;
  onLoadServerDetails: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onDisconnectConnection: (connectionId: string) => void;
  onReopenHistoryEntry: (entry: QueryHistoryEntry) => void;
};

function ConnectedOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  queryHistory,
  isConnected,
  overviewTab,
  relationStats,
  relationStatsStatus,
  serverDetails,
  serverDetailsStatus,
  onSetOverviewTab,
  onLoadStats,
  onLoadRelationStats,
  onLoadServerDetails,
  onOpenTable,
  onDisconnectConnection,
  onReopenHistoryEntry,
}: ConnectedOverviewProps) {
  const view = useDatabaseOverview({
    activeConnection,
    schemas,
    stats,
    statsStatus,
    queryHistory,
    isConnected,
    onLoadStats,
  });

  // Transient cross-tab filter — set by Schemas-tab row click, cleared
  // when the user leaves the Tables tab or clicks the chip. Lives in
  // local state because (a) it's a UX nicety, not durable settings,
  // and (b) it should reset between connections, which is implicit
  // here because ConnectedOverview is keyed by activeConnection.id.
  const [tablesSchemaFilter, setTablesSchemaFilter] = useState<string | null>(
    null,
  );

  const handleTabChange = (tab: OverviewTabId) => {
    if (tab !== "tables") {
      setTablesSchemaFilter(null);
    }
    onSetOverviewTab(activeConnection.id, tab);
  };

  const handleSelectSchemaFromSchemasTab = (schema: string) => {
    setTablesSchemaFilter(schema);
    onSetOverviewTab(activeConnection.id, "tables");
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex w-full max-w-384 flex-col gap-5 p-6">
          <OverviewHeader
            name={activeConnection.name}
            activeTab={overviewTab}
            onTabChange={handleTabChange}
            onDisconnect={() => onDisconnectConnection(activeConnection.id)}
          />

          <OverviewTabBody
            activeConnection={activeConnection}
            activeTab={overviewTab}
            view={view}
            statsStatus={statsStatus}
            queryHistory={queryHistory}
            schemas={schemas}
            relationStats={relationStats}
            relationStatsStatus={relationStatsStatus}
            serverDetails={serverDetails}
            serverDetailsStatus={serverDetailsStatus}
            tablesSchemaFilter={tablesSchemaFilter}
            onClearTablesSchemaFilter={() => setTablesSchemaFilter(null)}
            onSelectSchemaFromSchemasTab={handleSelectSchemaFromSchemasTab}
            onLoadRelationStats={onLoadRelationStats}
            onLoadServerDetails={onLoadServerDetails}
            onOpenTable={onOpenTable}
            onSwitchTab={handleTabChange}
            onReopenHistoryEntry={onReopenHistoryEntry}
          />
        </div>
      </div>
      <StatusBar items={view.overviewStatus} />
    </div>
  );
}

type OverviewTabBodyProps = {
  activeConnection: Connection;
  activeTab: OverviewTabId;
  view: ReturnType<typeof useDatabaseOverview>;
  statsStatus: DatabaseOverviewStatsStatus | undefined;
  queryHistory: QueryHistoryEntry[];
  schemas: SchemaExplorer[];
  relationStats: RelationInfo[] | undefined;
  relationStatsStatus: RelationStatsStatus | undefined;
  serverDetails: ServerDetails | undefined;
  serverDetailsStatus: ServerDetailsStatus | undefined;
  tablesSchemaFilter: string | null;
  onClearTablesSchemaFilter: () => void;
  onSelectSchemaFromSchemasTab: (schema: string) => void;
  onLoadRelationStats: (connectionId: string) => Promise<void>;
  onLoadServerDetails: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onSwitchTab: (tab: OverviewTabId) => void;
  onReopenHistoryEntry: (entry: QueryHistoryEntry) => void;
};

/**
 * Body region for the connection overview surface. Switches its
 * content based on the active sub-tab from the header. The Overview
 * sub-tab keeps the original four-card dashboard; the other Phase 1
 * sub-tabs render placeholders today and get filled in by their
 * respective Phase 1 steps. Engine-gated sub-tabs (Schemas, Details)
 * render the Postgres-only explainer for non-PG connections.
 */
function OverviewTabBody({
  activeConnection,
  activeTab,
  view,
  statsStatus,
  queryHistory,
  schemas,
  relationStats,
  relationStatsStatus,
  serverDetails,
  serverDetailsStatus,
  tablesSchemaFilter,
  onClearTablesSchemaFilter,
  onSelectSchemaFromSchemasTab,
  onLoadRelationStats,
  onLoadServerDetails,
  onOpenTable,
  onSwitchTab,
  onReopenHistoryEntry,
}: OverviewTabBodyProps) {
  const isPostgres = activeConnection.engine === "PostgreSQL";

  if (activeTab === "overview") {
    return (
      <>
        {/* Top row: Connection Details + Database Stats */}
        <section className="grid gap-4 lg:grid-cols-2">
          <ConnectionDetailsCard connection={activeConnection} />
          <DatabaseStatsCard
            tableCount={view.tableCount}
            schemaCount={view.schemaCount}
            databaseSize={view.databaseSize}
            indexes={view.indexCount}
            connections={view.connectionCount}
            rows={view.rowCount}
            statsStatus={statsStatus}
            rowCountKind={view.rowCountKind}
            onViewAll={() => onSwitchTab("schemas")}
          />
        </section>

        {/* Mid row: Recent Queries + Favorite Tables */}
        <section className="grid gap-4 lg:grid-cols-2">
          <RecentQueriesCard
            queries={view.recentQueries}
            onViewAll={() => onSwitchTab("query-history")}
          />
          <FavoriteTablesCard
            tables={view.favoriteTables}
            onOpenTable={onOpenTable}
            onViewAll={() => onSwitchTab("tables")}
          />
        </section>

        <HealthBanner connection={activeConnection} />
      </>
    );
  }

  if (activeTab === "tables") {
    return (
      <TablesTab
        activeConnection={activeConnection}
        schemas={schemas}
        relationStats={relationStats}
        relationStatsStatus={relationStatsStatus}
        schemaFilter={tablesSchemaFilter}
        onClearSchemaFilter={onClearTablesSchemaFilter}
        onLoadRelationStats={onLoadRelationStats}
        onOpenTable={onOpenTable}
      />
    );
  }

  if (activeTab === "schemas") {
    if (!isPostgres) {
      return (
        <PostgresOnlyPanel
          engine={activeConnection.engine}
          tabLabel="Schemas"
        />
      );
    }
    return (
      <SchemasTab
        activeConnection={activeConnection}
        relationStats={relationStats}
        relationStatsStatus={relationStatsStatus}
        onLoadRelationStats={onLoadRelationStats}
        onSelectSchema={onSelectSchemaFromSchemasTab}
      />
    );
  }

  if (activeTab === "query-history") {
    return (
      <QueryHistoryTab
        activeConnection={activeConnection}
        queryHistory={queryHistory}
        onReopenEntry={onReopenHistoryEntry}
      />
    );
  }

  if (activeTab === "details") {
    if (!isPostgres) {
      return (
        <PostgresOnlyPanel
          engine={activeConnection.engine}
          tabLabel="Details"
        />
      );
    }
    return (
      <DetailsTab
        activeConnection={activeConnection}
        details={serverDetails}
        status={serverDetailsStatus}
        onLoad={onLoadServerDetails}
      />
    );
  }

  // activeTab === "settings"
  return <SettingsTab connection={activeConnection} />;
}
