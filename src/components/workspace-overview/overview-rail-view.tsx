/**
 * Overview rail view (DESIGN-SYSTEM §5.7, D1/P9): the per-connection
 * landing — connection health, database stats, table catalog, and
 * recent queries as dense list rows composed from system primitives.
 * Reuses the salvaged `useDatabaseOverview` data hook; no stat-card
 * grids with 24px padding.
 */

import {
  IconClockHour3,
  IconDatabase,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useMemo } from "react";

import { EmptyState } from "@/components/ui/state-panel";
import { StatusDot } from "@/components/ui/status-dot";
import { useDatabaseOverview } from "@/components/workspace-overview/use-database-overview";
import {
  type Connection,
  type QueryHistoryEntry,
  type SchemaExplorer,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface OverviewRailViewProps {
  activeConnection: Connection;
  schemas: SchemaExplorer[];
  isConnected: boolean;
  onOpenTable: (schema: string, table: string) => void;
  onReopenQuery: (entry: { sql: string; connectionId: string }) => void;
}

const formatWhen = (iso: string): string => {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
};

const singleLineSql = (sql: string): string =>
  sql.replace(/\s+/g, " ").trim().slice(0, 140);

export function OverviewRailView({
  activeConnection,
  schemas,
  isConnected,
  onOpenTable,
  onReopenQuery,
}: OverviewRailViewProps) {
  const stats = useAppStore(
    (state) => state.databaseOverviewStats[activeConnection.id],
  );
  const statsStatus = useAppStore(
    (state) => state.databaseOverviewStatsStatus[activeConnection.id],
  );
  const relationStats = useAppStore(
    (state) => state.relationStats[activeConnection.id],
  );
  const relationStatsStatus = useAppStore(
    (state) => state.relationStatsStatus[activeConnection.id],
  );
  const queryHistory = useAppStore((state) => state.queryHistory);
  const loadDatabaseOverviewStats = useAppStore(
    (state) => state.loadDatabaseOverviewStats,
  );
  const loadRelationStats = useAppStore((state) => state.loadRelationStats);

  const connectionQueries = useMemo(
    () =>
      queryHistory.filter(
        (entry) => entry.connectionId === activeConnection.id,
      ),
    [queryHistory, activeConnection.id],
  );

  const view = useDatabaseOverview({
    activeConnection,
    schemas,
    stats,
    statsStatus,
    queryHistory: connectionQueries,
    isConnected,
    onLoadStats: loadDatabaseOverviewStats,
    relationStats,
    relationStatsStatus,
    onLoadRelationStats: loadRelationStats,
  });

  const tableCatalog = useMemo(() => {
    const rowCounts = new Map(
      (relationStats ?? []).map((row) => [
        `${row.schema}.${row.name}`,
        row.rowCountEstimate,
      ]),
    );
    return schemas.flatMap((schema) =>
      schema.tables.map((table) => ({
        schema: schema.name,
        name: table,
        rowCount: rowCounts.get(`${schema.name}.${table}`) ?? null,
      })),
    );
  }, [schemas, relationStats]);

  const statRows: Array<{ label: string; value: string }> = [
    { label: "Tables", value: String(view.tableCount) },
    { label: "Schemas", value: String(view.schemaCount) },
    { label: "Database size", value: view.databaseSize },
    {
      label: view.rowCountKind === "exact" ? "Rows" : "Rows (estimate)",
      value: view.rowCount,
    },
    { label: "Indexes", value: view.indexCount },
    { label: "Active connections", value: view.connectionCount },
  ];

  return (
    <div
      data-testid="overview-rail-view"
      className="flex min-h-0 flex-1 flex-col overflow-auto bg-surface-app"
    >
      {/* Health header */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-2">
        <IconDatabase className="size-4 shrink-0 text-text-muted" />
        <span className="truncate text-sm font-semibold text-foreground">
          {activeConnection.name}
        </span>
        <span className="truncate text-xs text-text-muted">
          {activeConnection.engine} · {activeConnection.database}
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-1.5 text-xs text-text-secondary">
          <StatusDot
            tone={isConnected ? "healthy" : "neutral"}
            className="size-1.5"
          />
          {isConnected ? "Connected" : "Disconnected"}
          {activeConnection.latency && activeConnection.latency !== "--"
            ? ` · ${activeConnection.latency}`
            : ""}
        </span>
      </div>

      <div className="grid min-h-0 flex-1 grid-cols-1 gap-x-6 px-3 py-2 lg:grid-cols-2">
        <div className="min-w-0">
          <OverviewSection title="Database">
            <ul>
              {statRows.map((row) => (
                <li
                  key={row.label}
                  className="flex h-(--row-tree) items-center justify-between gap-3 border-b border-border-subtle/50 text-xs last:border-b-0"
                >
                  <span className="text-text-muted">{row.label}</span>
                  <span className="font-mono tabular-nums text-text-secondary">
                    {row.value}
                  </span>
                </li>
              ))}
            </ul>
          </OverviewSection>

          <OverviewSection title={`Tables · ${tableCatalog.length}`}>
            {tableCatalog.length === 0 ? (
              <EmptyState
                title={
                  isConnected ? "No tables loaded" : "Connect to load tables"
                }
                className="h-auto py-4"
              />
            ) : (
              <ul>
                {tableCatalog.map((table) => (
                  <li key={`${table.schema}.${table.name}`}>
                    <button
                      type="button"
                      onClick={() => onOpenTable(table.schema, table.name)}
                      className="flex h-(--row-tree) w-full items-center gap-2 rounded-sm px-1 text-left text-xs text-text-secondary hover:bg-surface-row-hover hover:text-foreground"
                    >
                      <IconTable className="size-3.5 shrink-0 text-text-disabled" />
                      <span className="truncate">
                        <span className="text-text-muted">{table.schema}.</span>
                        {table.name}
                      </span>
                      {table.rowCount !== null ? (
                        <span className="ml-auto shrink-0 font-mono text-2xs tabular-nums text-text-muted">
                          {table.rowCount.toLocaleString()}
                        </span>
                      ) : null}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </OverviewSection>
        </div>

        <div className="min-w-0">
          <OverviewSection title="Recent queries">
            {connectionQueries.length === 0 ? (
              <EmptyState title="No queries yet" className="h-auto py-4" />
            ) : (
              <ul>
                {connectionQueries.slice(0, 20).map((entry) => (
                  <RecentQueryRow
                    key={entry.id}
                    entry={entry}
                    onReopen={() =>
                      onReopenQuery({
                        sql: entry.sql,
                        connectionId: entry.connectionId,
                      })
                    }
                  />
                ))}
              </ul>
            )}
          </OverviewSection>
        </div>
      </div>
    </div>
  );
}

function OverviewSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="py-2">
      <h2 className="mb-1 text-2xs font-semibold tracking-[0.12em] text-text-muted uppercase">
        {title}
      </h2>
      {children}
    </section>
  );
}

function RecentQueryRow({
  entry,
  onReopen,
}: {
  entry: QueryHistoryEntry;
  onReopen: () => void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onReopen}
        className="flex w-full items-start gap-2 rounded-sm px-1 py-1 text-left text-xs hover:bg-surface-row-hover"
      >
        <IconTerminal2
          className={cn(
            "mt-0.5 size-3.5 shrink-0",
            entry.status === "error" ? "text-danger" : "text-text-disabled",
          )}
        />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-mono text-text-secondary">
            {singleLineSql(entry.sql)}
          </span>
          <span className="mt-0.5 flex items-center gap-2 text-2xs text-text-muted">
            <IconClockHour3 className="size-3" />
            {formatWhen(entry.startedAt)}
            <span className="tabular-nums">{entry.runtimeMs} ms</span>
            {entry.status === "error" ? (
              <span className="text-danger">failed</span>
            ) : null}
          </span>
        </span>
      </button>
    </li>
  );
}
