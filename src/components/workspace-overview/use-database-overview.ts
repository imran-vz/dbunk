import { useEffect, useMemo } from "react";

import type { StatusBarItem } from "@/components/status-bar";
import { relationalPolicy } from "@/lib/engine-policy";
import type {
  Connection,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  QueryHistoryEntry,
  SchemaExplorer,
} from "@/lib/store";

import { formatByteStat, formatRowCount, metricCount } from "./format";

interface UseDatabaseOverviewArgs {
  activeConnection: Connection;
  schemas: SchemaExplorer[];
  stats: DatabaseOverviewStats | undefined;
  statsStatus: DatabaseOverviewStatsStatus | undefined;
  queryHistory: QueryHistoryEntry[];
  isConnected: boolean;
  onLoadStats: (connectionId: string) => Promise<void>;
}

interface FavoriteTable {
  schema: string;
  name: string;
}

export interface DatabaseOverviewView {
  tableCount: number;
  schemaCount: number;
  databaseSize: string;
  rowCount: string;
  indexCount: string;
  connectionCount: string;
  recentQueries: QueryHistoryEntry[];
  favoriteTables: FavoriteTable[];
  overviewStatus: StatusBarItem[];
  rowCountKind: "estimate" | "exact";
}

/**
 * Owns the store-derived view for `WorkspaceDatabaseOverview`:
 * - Lazily kicks off `loadDatabaseOverviewStats` when the connection is
 *   live but stats are absent.
 * - Flattens `SchemaExplorer[]` → favorite-table candidates and slices
 *   the top entries for the dashboard.
 * - Prefers server-side stats and falls back to local schemas on first
 *   paint so the dashboard isn't blank while stats stream in.
 */
export function useDatabaseOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  queryHistory,
  isConnected,
  onLoadStats,
}: UseDatabaseOverviewArgs): DatabaseOverviewView {
  const tables = useMemo(
    () =>
      schemas.flatMap((schema) =>
        schema.tables.map((table) => ({
          schema: schema.name,
          name: table,
        })),
      ),
    [schemas],
  );

  useEffect(() => {
    if (isConnected && !stats && statsStatus?.state !== "loading") {
      void onLoadStats(activeConnection.id);
    }
  }, [
    activeConnection.id,
    isConnected,
    onLoadStats,
    stats,
    statsStatus?.state,
  ]);

  const tableCount = stats?.tableCount ?? tables.length;
  const schemaCount = stats?.schemaCount ?? schemas.length;
  const databaseSize = formatByteStat(stats?.databaseSizeBytes, statsStatus);
  const rowCount = formatRowCount(stats?.rowCountEstimate, statsStatus);
  const indexCount = metricCount(stats?.indexCount, statsStatus);
  const connectionCount = metricCount(stats?.connectionCount, statsStatus);

  const recentQueries = queryHistory.slice(0, 4);
  const favoriteTables = tables.slice(0, 5);
  const lastQuery = queryHistory[0];

  const overviewStatus: StatusBarItem[] = [
    {
      id: "checked",
      tone: "healthy",
      label: "Health",
      value: "All checks passing",
    },
    {
      id: "engine",
      label: "Engine",
      value: activeConnection.engine,
    },
    {
      id: "latency",
      label: "Latency",
      value: lastQuery ? `${lastQuery.runtimeMs} ms` : "—",
      align: "right",
    },
    {
      id: "connection",
      label: "Connection",
      value: activeConnection.status,
      tone: "healthy",
      align: "right",
    },
  ];

  return {
    tableCount,
    schemaCount,
    databaseSize,
    rowCount,
    indexCount,
    connectionCount,
    recentQueries,
    favoriteTables,
    overviewStatus,
    rowCountKind: relationalPolicy(activeConnection.engine).rowCountKind,
  };
}
