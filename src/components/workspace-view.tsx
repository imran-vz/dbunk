import { useMemo } from "react";

import { KeyValueWorkspace } from "@/components/keyvalue/KeyValueWorkspace";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import { StatusBar } from "@/components/status-bar";
import { TableEditorPanel } from "@/components/table-editor-panel";
import { ConnectionDetailsCard } from "@/components/workspace-overview/connection-details-card";
import { DatabaseStatsCard } from "@/components/workspace-overview/database-stats-card";
import {
  DisconnectedConnectionCard,
  NoConnectionCard,
} from "@/components/workspace-overview/disconnected-card";
import { FavoriteTablesCard } from "@/components/workspace-overview/favorite-tables-card";
import { HealthBanner } from "@/components/workspace-overview/health-banner";
import { OverviewHeader } from "@/components/workspace-overview/overview-header";
import { RecentQueriesCard } from "@/components/workspace-overview/recent-queries-card";
import { useDatabaseOverview } from "@/components/workspace-overview/use-database-overview";
import { WorkspaceTabs } from "@/components/workspace-tabs";
import { storageClassFor } from "@/lib/engine-policy";
import {
  type Connection,
  type DatabaseOverviewStats,
  type DatabaseOverviewStatsStatus,
  type QueryHistoryEntry,
  type SchemaExplorer,
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
    databaseOverviewStats,
    databaseOverviewStatsStatus,
    queryHistory,
    schemaExplorer,
    workspaceTabs,
    createNewQueryTab,
    connectConnection,
    disconnectConnection,
    loadDatabaseOverviewStats,
    openTableTab,
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
            onLoadStats={loadDatabaseOverviewStats}
            onOpenTable={openTableTab}
            onNewQuery={createNewQueryTab}
            onConnectConnection={connectConnection}
            onDisconnectConnection={disconnectConnection}
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
  onLoadStats: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onNewQuery: () => void;
  onConnectConnection: (connectionId: string) => Promise<void>;
  onDisconnectConnection: (connectionId: string) => void;
};

function WorkspaceDatabaseOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  queryHistory,
  onLoadStats,
  onOpenTable,
  onNewQuery,
  onConnectConnection,
  onDisconnectConnection,
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
      onLoadStats={onLoadStats}
      onOpenTable={onOpenTable}
      onDisconnectConnection={onDisconnectConnection}
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
  onLoadStats: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onDisconnectConnection: (connectionId: string) => void;
};

function ConnectedOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  queryHistory,
  isConnected,
  onLoadStats,
  onOpenTable,
  onDisconnectConnection,
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

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex w-full max-w-384 flex-col gap-5 p-6">
          <OverviewHeader
            name={activeConnection.name}
            onDisconnect={() => onDisconnectConnection(activeConnection.id)}
          />

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
            />
          </section>

          {/* Mid row: Recent Queries + Favorite Tables */}
          <section className="grid gap-4 lg:grid-cols-2">
            <RecentQueriesCard queries={view.recentQueries} />
            <FavoriteTablesCard
              tables={view.favoriteTables}
              onOpenTable={onOpenTable}
            />
          </section>

          <HealthBanner connection={activeConnection} />
        </div>
      </div>
      <StatusBar items={view.overviewStatus} />
    </div>
  );
}
