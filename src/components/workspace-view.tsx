import {
  IconActivityHeartbeat,
  IconArrowRight,
  IconClock,
  IconDatabase,
  IconEdit,
  IconRefresh,
  IconStar,
  IconTerminal2,
} from "@tabler/icons-react";
import { useEffect, useMemo } from "react";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import { StatusBar } from "@/components/status-bar";
import { TableEditorPanel } from "@/components/table-editor-panel";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { StatusDot } from "@/components/ui/status-dot";
import { WorkspaceTabs } from "@/components/workspace-tabs";
import { enginePolicy } from "@/lib/engine-policy";
import {
  type Connection,
  type DatabaseOverviewStats,
  type DatabaseOverviewStatsStatus,
  type QueryHistoryEntry,
  type SchemaExplorer,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface WorkspaceViewProps {
  isClient: boolean;
}

const OVERVIEW_TABS = [
  "Overview",
  "Tables",
  "Schemas",
  "Query History",
  "Details",
  "Settings",
] as const;

const formatConnectionLatency = (latency: unknown) => {
  if (typeof latency !== "string") {
    return null;
  }
  const normalized = latency.trim().toLowerCase();
  if (
    normalized === "" ||
    normalized === "--" ||
    normalized === "undefined ms" ||
    normalized === "null ms" ||
    normalized === "nan ms"
  ) {
    return null;
  }
  return latency;
};

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

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <WorkspaceTabs />
      <div className="flex min-h-0 flex-1 flex-col bg-surface-app">
        {activeTab ? (
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
          />
        )}
      </div>
    </div>
  );
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
}: WorkspaceDatabaseOverviewProps) {
  const isConnected =
    activeConnection?.status === "Connected" ||
    activeConnection?.status === "Read only";

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
    if (
      activeConnection &&
      isConnected &&
      !stats &&
      statsStatus?.state !== "loading"
    ) {
      void onLoadStats(activeConnection.id);
    }
  }, [activeConnection, isConnected, onLoadStats, stats, statsStatus?.state]);

  if (!activeConnection) {
    return (
      <div className="flex flex-1 items-center justify-center p-6">
        <Card className="max-w-md">
          <CardHeader>
            <CardTitle>No database connected</CardTitle>
          </CardHeader>
          <CardContent className="text-xs text-text-muted">
            Connect a database or create a new query to begin.
          </CardContent>
        </Card>
      </div>
    );
  }

  if (!isConnected && schemas.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center p-6">
        <Card className="w-full max-w-xl">
          <CardHeader>
            <CardTitle>{activeConnection.name}</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-xs">
            <div className="flex items-center gap-2 text-text-muted">
              <StatusDot tone="neutral" />
              <span>{activeConnection.status}</span>
              <span>·</span>
              <span>{activeConnection.engine}</span>
              <span>·</span>
              <span>{activeConnection.database}</span>
            </div>
            <div className="grid grid-cols-2 gap-3 rounded-md border border-border-subtle bg-surface-panel p-3">
              <KeyValue
                label="Host"
                value={`${activeConnection.host || "--"}:${
                  activeConnection.port || "--"
                }`}
              />
              <KeyValue label="User" value={activeConnection.user || "--"} />
            </div>
            {activeConnection.errorMessage ? (
              <div className="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-danger">
                {activeConnection.errorMessage}
              </div>
            ) : null}
            <div className="flex items-center justify-end gap-2">
              <Button size="sm" variant="outline" onClick={onNewQuery}>
                <IconTerminal2 className="size-3.5" />
                New Query
              </Button>
              <Button
                size="sm"
                onClick={() => {
                  void onConnectConnection(activeConnection.id);
                }}
              >
                <IconDatabase className="size-3.5" />
                Connect
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    );
  }

  // Prefer the server-side counts from `load_database_overview_stats` and fall
  // back to the locally known schemas when stats haven't loaded yet — keeps
  // the dashboard responsive on first paint.
  const tableCount = stats?.tableCount ?? tables.length;
  const schemaCount = stats?.schemaCount ?? schemas.length;
  const databaseSize = formatByteStat(stats?.databaseSizeBytes, statsStatus);
  const rowCount = formatRowCount(stats?.rowCountEstimate, statsStatus);
  const indexCount =
    stats?.indexCount !== undefined
      ? stats.indexCount.toLocaleString()
      : pendingMetric(statsStatus);
  const connectionCount =
    stats?.connectionCount !== undefined
      ? stats.connectionCount.toLocaleString()
      : pendingMetric(statsStatus);

  const recentQueries = queryHistory.slice(0, 4);
  const favoriteTables = tables.slice(0, 5);

  const lastQuery = queryHistory[0];
  const overviewStatus = [
    {
      id: "checked",
      tone: "healthy" as const,
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
      align: "right" as const,
    },
    {
      id: "connection",
      label: "Connection",
      value: activeConnection.status,
      tone: "healthy" as const,
      align: "right" as const,
    },
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto flex w-full max-w-[96rem] flex-col gap-5 p-6">
          {/* Header */}
          <header className="flex flex-wrap items-end justify-between gap-3 border-b border-border-subtle pb-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2.5">
                <h1 className="text-xl font-semibold tracking-tight text-foreground">
                  {activeConnection.name}
                </h1>
                <span className="flex items-center gap-1.5 text-xs font-medium text-accent-green-hover">
                  <StatusDot tone="healthy" className="size-2" />
                  Connected
                </span>
              </div>
              <nav
                aria-label="Connection sections"
                className="mt-3 flex flex-wrap items-center gap-1 text-xs"
              >
                {OVERVIEW_TABS.map((label, index) => {
                  const isActive = index === 0;
                  return (
                    <button
                      key={label}
                      type="button"
                      aria-current={isActive ? "page" : undefined}
                      className={cn(
                        "rounded-md px-2.5 py-1 transition-colors",
                        isActive
                          ? "bg-accent-green/10 text-accent-green-hover"
                          : "text-text-muted hover:bg-surface-panel hover:text-foreground",
                      )}
                    >
                      {label}
                    </button>
                  );
                })}
              </nav>
            </div>
          </header>

          {/* Top row: Connection Details + Database Stats */}
          <section className="grid gap-4 lg:grid-cols-2">
            <ConnectionDetailsCard connection={activeConnection} />
            <DatabaseStatsCard
              tableCount={tableCount}
              schemaCount={schemaCount}
              databaseSize={databaseSize}
              indexes={indexCount}
              connections={connectionCount}
              rows={rowCount}
              statsStatus={statsStatus}
              rowCountKind={enginePolicy(activeConnection.engine).rowCountKind}
            />
          </section>

          {/* Mid row: Recent Queries + Favorite Tables */}
          <section className="grid gap-4 lg:grid-cols-2">
            <RecentQueriesCard queries={recentQueries} />
            <FavoriteTablesCard
              tables={favoriteTables}
              onOpenTable={onOpenTable}
            />
          </section>

          <HealthBanner connection={activeConnection} />
        </div>
      </div>
      <StatusBar items={overviewStatus} />
    </div>
  );
}

function HealthBanner({ connection }: { connection: Connection }) {
  const status = connection.status;
  const isHealthy = status === "Connected" || status === "Read only";
  const lastChecked = formatLastChecked(connection.lastSync);
  const latency = formatConnectionLatency(connection.latency);

  if (isHealthy) {
    return (
      <section className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-accent-green/25 bg-accent-green-subdued/40 px-4 py-3">
        <div className="flex items-center gap-3">
          <span className="flex size-9 items-center justify-center rounded-md bg-accent-green/15 text-accent-green">
            <IconActivityHeartbeat className="size-5" />
          </span>
          <div>
            <div className="text-sm font-medium text-accent-green-hover">
              Your connection is healthy
            </div>
            <div className="text-xs text-accent-green-hover/70">
              {latency
                ? `Round-trip ${latency}. Last checked ${lastChecked}.`
                : `Last checked ${lastChecked}.`}
            </div>
          </div>
        </div>
        <Button size="sm" variant="outline">
          View health checks
        </Button>
      </section>
    );
  }

  return (
    <section className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3">
      <div className="flex items-center gap-3">
        <span className="flex size-9 items-center justify-center rounded-md bg-danger/15 text-danger">
          <IconActivityHeartbeat className="size-5" />
        </span>
        <div>
          <div className="text-sm font-medium text-danger">
            Connection unreachable
          </div>
          <div className="font-mono text-[0.6875rem] text-danger/80">
            {connection.errorMessage ?? "Last health check failed."}
          </div>
        </div>
      </div>
      <Button size="sm" variant="outline">
        View health checks
      </Button>
    </section>
  );
}

function formatLastChecked(value: string | undefined): string {
  if (!value || value === "Never" || value === "Just now") {
    return value === "Never" ? "never" : "just now";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 60_000) return "just now";
  if (diffMs < 3_600_000) {
    const minutes = Math.round(diffMs / 60_000);
    return `${minutes} min ago`;
  }
  if (diffMs < 86_400_000) {
    const hours = Math.round(diffMs / 3_600_000);
    return `${hours} hr ago`;
  }
  return date.toLocaleString();
}

function ConnectionDetailsCard({ connection }: { connection: Connection }) {
  const rows: Array<[string, React.ReactNode]> = [
    ["Host", connection.host || "prod-db.dbunk.io"],
    ["Database", connection.database || "app_prod"],
    ["User", connection.user || "dbunk_app"],
    ["Engine", connection.engine],
    ["Region", "us-east-1"],
    ["SSL", "Enabled"],
  ];

  return (
    <Card>
      <CardHeader>
        <CardTitle>Connection Details</CardTitle>
        <CardAction>
          <Button size="sm" variant="outline">
            <IconEdit className="size-3.5" />
            Edit Connection
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="grid grid-cols-2 gap-x-6 gap-y-3 text-xs">
        {rows.map(([label, value]) => (
          <KeyValue key={label} label={label} value={value} />
        ))}
      </CardContent>
    </Card>
  );
}

type DatabaseStatsCardProps = {
  tableCount: number;
  schemaCount: number;
  databaseSize: string;
  indexes: string;
  connections: string;
  rows: string;
  statsStatus?: DatabaseOverviewStatsStatus;
  /**
   * "estimate" — PG planner estimate from `pg_class.reltuples`.
   * "exact"    — ClickHouse aggregate from `system.parts.rows`.
   *
   * Drives the "Rows" label so users know what kind of number they're
   * looking at (relevant when the same dashboard is rendered for
   * different engines).
   */
  rowCountKind: "estimate" | "exact";
};

function DatabaseStatsCard({
  tableCount,
  schemaCount,
  databaseSize,
  indexes,
  connections,
  rows,
  statsStatus,
  rowCountKind,
}: DatabaseStatsCardProps) {
  const rowsLabel = rowCountKind === "estimate" ? "Rows (≈)" : "Rows";
  const metrics: Array<[string, React.ReactNode]> = [
    ["Tables", tableCount.toLocaleString()],
    ["Schemas", schemaCount.toLocaleString()],
    [rowsLabel, rows],
    ["Size", databaseSize],
    ["Indexes", indexes],
    ["Connections", connections],
  ];
  const refreshing = statsStatus?.state === "loading";

  return (
    <Card>
      <CardHeader>
        <CardTitle>Database Stats</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost" disabled={refreshing}>
            <IconRefresh
              className={cn("size-3.5", refreshing && "animate-spin")}
            />
            {refreshing ? "Refreshing…" : "Updated just now"}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="grid grid-cols-3 gap-x-4 gap-y-4 text-xs">
        {metrics.map(([label, value]) => (
          <div key={label}>
            <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
              {label}
            </div>
            <div className="mt-1 text-lg font-semibold tabular-nums text-foreground">
              {value}
            </div>
          </div>
        ))}
      </CardContent>
      <div className="px-4 pt-1">
        <Button size="sm" variant="ghost" className="px-1 text-text-muted">
          View all metrics
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}

function RecentQueriesCard({ queries }: { queries: QueryHistoryEntry[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Queries</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost">
            View all
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-2 text-xs">
        {queries.length === 0 ? (
          <div className="flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-border-subtle px-3 py-6 text-text-muted">
            <IconTerminal2 className="size-5 opacity-60" />
            <span>No queries yet — open the editor and run one.</span>
          </div>
        ) : (
          queries.map((query) => (
            <div
              key={query.id}
              className="flex items-center gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2"
            >
              <span
                className={cn(
                  "flex size-7 items-center justify-center rounded-md",
                  query.status === "success"
                    ? "bg-accent-green/10 text-accent-green-hover"
                    : "bg-danger/10 text-danger",
                )}
              >
                <IconTerminal2 className="size-3.5" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate font-mono text-[0.75rem] text-foreground">
                  {query.sql}
                </div>
                <div className="mt-0.5 flex items-center gap-1.5 text-[0.625rem] text-text-muted">
                  <IconClock className="size-2.5" />
                  <span>{query.startedAt}</span>
                  <span>·</span>
                  <span className="truncate">{query.connectionName}</span>
                </div>
              </div>
              <span className="rounded-full bg-accent-green/10 px-2 py-0.5 text-[0.625rem] font-medium tabular-nums text-accent-green-hover">
                {query.runtimeMs} ms
              </span>
            </div>
          ))
        )}
      </CardContent>
      <div className="px-4 pt-1">
        <Button size="sm" variant="ghost" className="px-1 text-text-muted">
          Open Query History
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}

function FavoriteTablesCard({
  tables,
  onOpenTable,
}: {
  tables: Array<{ schema: string; name: string }>;
  onOpenTable: (schema: string, table: string) => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Favorite Tables</CardTitle>
        <CardAction>
          <Button size="sm" variant="ghost">
            Manage
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-1.5 text-xs">
        {tables.length === 0 ? (
          <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
            No favorites yet — star a table to pin it here.
          </div>
        ) : (
          tables.map((table) => (
            <button
              key={`${table.schema}.${table.name}`}
              type="button"
              onClick={() => onOpenTable(table.schema, table.name)}
              className="flex items-center gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2 text-left transition-colors hover:bg-surface-row-hover"
            >
              <IconStar className="size-3.5 text-warning" />
              <span className="min-w-0 flex-1 truncate font-mono text-[0.75rem] text-foreground">
                {table.name}
              </span>
              <span className="text-[0.625rem] text-text-muted">
                {table.schema}
              </span>
              <span className="text-[0.625rem] tabular-nums text-text-muted">
                {/* TODO(Phase 4 follow-up): real row counts */}
                {/* eslint-disable-next-line no-magic-numbers */}—
              </span>
            </button>
          ))
        )}
      </CardContent>
      <div className="px-4 pt-1">
        <Button size="sm" variant="ghost" className="px-1 text-text-muted">
          Browse all tables
          <IconArrowRight className="size-3" />
        </Button>
      </div>
    </Card>
  );
}

function KeyValue({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="min-w-0">
      <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
        {label}
      </div>
      <div className="mt-0.5 truncate text-foreground">{value}</div>
    </div>
  );
}

function formatByteStat(
  value: number | undefined,
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  if (typeof value === "number") {
    return formatBytes(value);
  }
  if (status?.state === "loading") {
    return "…";
  }
  if (status?.state === "error") {
    return "Unavailable";
  }
  return "—";
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision =
    Number.isInteger(value) || value >= 10 || unitIndex === 0 ? 0 : 1;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

function pendingMetric(
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  if (status?.state === "loading") return "…";
  if (status?.state === "error") return "Unavailable";
  return "—";
}

// PG returns reltuples as a planner estimate — use compact suffixes for the
// dashboard (1.2M, 18.2K). Precise counts would need per-table SELECT count(*)
// which can be expensive on large databases.
function formatRowCount(
  value: number | undefined,
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return pendingMetric(status);
  }
  if (value < 1000) return value.toLocaleString();
  const units: Array<[number, string]> = [
    [1_000_000_000, "B"],
    [1_000_000, "M"],
    [1_000, "K"],
  ];
  for (const [scale, suffix] of units) {
    if (value >= scale) {
      const scaled = value / scale;
      const precision = scaled >= 10 ? 0 : 1;
      return `${scaled.toFixed(precision)}${suffix}`;
    }
  }
  return value.toLocaleString();
}
