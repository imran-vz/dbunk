import {
  IconDatabase,
  IconLayoutSidebarRight,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import { QueryEditorPanel } from "@/components/query-editor-panel";
import { QuerySidebar } from "@/components/query-sidebar";
import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import {
  TableEditorPanel,
  TableSidebar,
} from "@/components/table-editor-panel";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { WorkspaceTabs } from "@/components/workspace-tabs";
import {
  type Connection,
  type DatabaseOverviewStats,
  type DatabaseOverviewStatsStatus,
  type SchemaExplorer,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface WorkspaceViewProps {
  isClient: boolean;
}

export function WorkspaceView({ isClient }: WorkspaceViewProps) {
  const [isSidebarOpen, setIsSidebarOpen] = useState(false);

  const {
    activeConnectionId,
    activeTabId,
    connections,
    databaseOverviewStats,
    databaseOverviewStatsStatus,
    schemaExplorer,
    workspaceTabs,
    createNewQueryTab,
    createNewTableTab,
    loadDatabaseOverviewStats,
    openTableTab,
    openViewTab,
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
    <>
      <header className="flex h-12 items-center justify-between border-b px-3">
        <div className="flex items-center gap-2">
          <div className="text-sm font-semibold">Workspace</div>
          <div className="text-xs text-muted-foreground">
            {activeConnection
              ? `/ ${activeConnection.name} / ${activeConnection.database}`
              : "/ No active connection"}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="secondary" onClick={createNewQueryTab}>
            <IconTerminal2 />
            New query
          </Button>
          <Button size="sm" variant="outline" onClick={createNewTableTab}>
            <IconTable />
            New table
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="size-8"
            onClick={() => setIsSidebarOpen(!isSidebarOpen)}
          >
            <IconLayoutSidebarRight className="size-4" />
          </Button>
        </div>
      </header>

      <WorkspaceTabs />

      <div className="flex min-h-0 flex-1">
        <div
          className={cn(
            "grid h-full min-h-0 flex-1 transition-all duration-300 ease-in-out",
            isSidebarOpen
              ? "grid-cols-[minmax(0,1fr)_320px]"
              : "grid-cols-[minmax(0,1fr)_0px]",
          )}
        >
          <section className="flex min-h-0 min-w-0 flex-col border-r">
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
                isClient={isClient}
                onLoadStats={loadDatabaseOverviewStats}
                onOpenTable={openTableTab}
                onOpenView={openViewTab}
                onNewQuery={createNewQueryTab}
              />
            )}
          </section>

          <aside
            className={cn(
              "flex min-h-0 flex-col overflow-auto bg-muted/20 transition-all duration-300 ease-in-out",
              isSidebarOpen ? "p-3 gap-2 opacity-100" : "w-0 p-0 opacity-0",
            )}
          >
            {activeTab ? (
              activeTab.kind === "query" ? (
                <QuerySidebar tab={activeTab} />
              ) : (
                <TableSidebar tab={activeTab} isClient={isClient} />
              )
            ) : (
              <Card size="sm" className="border border-border">
                <CardHeader>
                  <CardTitle>Workspace tips</CardTitle>
                </CardHeader>
                <CardContent className="text-xs text-muted-foreground">
                  Use the schema explorer to open a table or start a new query
                  from the toolbar.
                </CardContent>
              </Card>
            )}
          </aside>
        </div>
      </div>
    </>
  );
}

type WorkspaceDatabaseOverviewProps = {
  activeConnection?: Connection;
  schemas: SchemaExplorer[];
  stats?: DatabaseOverviewStats;
  statsStatus?: DatabaseOverviewStatsStatus;
  isClient: boolean;
  onLoadStats: (connectionId: string) => Promise<void>;
  onOpenTable: (schemaName: string, tableName: string) => void;
  onOpenView: (schemaName: string, viewName: string) => void;
  onNewQuery: () => void;
};

function WorkspaceDatabaseOverview({
  activeConnection,
  schemas,
  stats,
  statsStatus,
  isClient,
  onLoadStats,
  onOpenTable,
  onOpenView,
  onNewQuery,
}: WorkspaceDatabaseOverviewProps) {
  const [selectedSchema, setSelectedSchema] = useState(schemas[0]?.name ?? "");

  useEffect(() => {
    if (!schemas.some((schema) => schema.name === selectedSchema)) {
      setSelectedSchema(schemas[0]?.name ?? "");
    }
  }, [schemas, selectedSchema]);

  const tables = useMemo(
    () =>
      schemas.flatMap((schema) =>
        schema.tables.map((table) => ({
          kind: "table" as const,
          schema: schema.name,
          name: table,
        })),
      ),
    [schemas],
  );
  const views = useMemo(
    () =>
      schemas.flatMap((schema) =>
        (schema.views ?? []).map((view) => ({
          kind: "view" as const,
          schema: schema.name,
          name: view,
        })),
      ),
    [schemas],
  );
  const selectedSchemaData =
    schemas.find((schema) => schema.name === selectedSchema) ?? schemas[0];
  const isConnected =
    activeConnection?.status === "Connected" ||
    activeConnection?.status === "Read only";

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

  if (!activeConnection || (!isConnected && schemas.length === 0)) {
    return (
      <div className="flex flex-1 items-center justify-center p-6">
        <Card className="border border-border">
          <CardHeader>
            <CardTitle>No database connected</CardTitle>
          </CardHeader>
          <CardContent className="text-xs text-muted-foreground">
            Connect a database or create a new query to begin.
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className="min-h-0 flex-1 overflow-auto bg-muted/10">
      <div className="mx-auto flex w-full max-w-[96rem] flex-col gap-3 p-3 sm:p-4">
        <section className="flex flex-wrap items-center justify-between gap-3 border-b bg-background px-3 py-3 sm:px-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-sm font-semibold">
              <IconDatabase className="size-4 text-primary" />
              {activeConnection.name}
            </div>
            <div className="truncate text-xs text-muted-foreground">
              {activeConnection.engine} / {activeConnection.database}
            </div>
          </div>
          <Button size="sm" variant="secondary" onClick={onNewQuery}>
            <IconTerminal2 className="size-3.5" />
            New query
          </Button>
        </section>

        <section className="grid grid-cols-[repeat(auto-fit,minmax(10rem,1fr))] gap-3">
          <StatCard label="Schemas" value={schemas.length} />
          <StatCard label="Tables" value={tables.length} />
          <StatCard label="Views" value={views.length} />
          <StatCard
            label="Database size"
            value={formatByteStat(stats?.databaseSizeBytes, statsStatus)}
          />
          <StatCard
            label="Table size"
            value={formatByteStat(stats?.tableSizeBytes, statsStatus)}
          />
          <StatCard
            label="Index size"
            value={formatByteStat(stats?.indexSizeBytes, statsStatus)}
          />
          <StatCard label="Status" value={activeConnection.status} />
        </section>

        <section className="grid min-h-0 gap-3 xl:grid-cols-[minmax(12rem,16rem)_minmax(24rem,1fr)] min-[1900px]:grid-cols-[16rem_minmax(34rem,1fr)_22rem]">
          <Card className="min-h-0 border border-border xl:h-[clamp(22rem,calc(100vh-19rem),42rem)]">
            <CardHeader>
              <CardTitle>Schemas</CardTitle>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-auto text-xs">
              <div className="flex flex-col gap-1">
                {schemas.map((schema) => {
                  const isSelected = schema.name === selectedSchemaData?.name;
                  return (
                    <button
                      type="button"
                      key={schema.name}
                      aria-label={`Select schema ${schema.name}`}
                      onClick={() => setSelectedSchema(schema.name)}
                      className={cn(
                        "flex items-center justify-between rounded-md border px-2 py-2 text-left transition",
                        isSelected
                          ? "border-primary bg-primary/10 text-primary"
                          : "border-transparent hover:border-border hover:bg-muted/40",
                      )}
                    >
                      <span className="truncate font-medium">
                        {schema.name}
                      </span>
                      <span className="text-muted-foreground">
                        {schema.tables.length + (schema.views?.length ?? 0)}
                      </span>
                    </button>
                  );
                })}
              </div>
            </CardContent>
          </Card>

          <Card className="min-h-0 border border-border xl:h-[clamp(22rem,calc(100vh-19rem),42rem)]">
            <CardHeader>
              <CardTitle>Tables schema map</CardTitle>
            </CardHeader>
            <CardContent className="min-h-0 flex-1">
              {selectedSchemaData ? (
                <div className="h-[clamp(22rem,calc(100vh-24rem),38rem)] min-h-0 overflow-hidden rounded-md border bg-background xl:h-full">
                  <SchemaRelationshipMap
                    connectionId={activeConnection.id}
                    schema={selectedSchemaData.name}
                    activeTable={null}
                    isClient={isClient}
                  />
                </div>
              ) : (
                <div className="flex h-[clamp(22rem,calc(100vh-24rem),38rem)] items-center justify-center rounded-md border text-xs text-muted-foreground xl:h-full">
                  No schema selected
                </div>
              )}
            </CardContent>
          </Card>

          <Card className="min-h-0 border border-border xl:col-span-2 min-[1900px]:col-span-1 min-[1900px]:h-[clamp(22rem,calc(100vh-19rem),42rem)]">
            <CardHeader>
              <CardTitle>Tables and views</CardTitle>
            </CardHeader>
            <CardContent className="min-h-0 flex-1 overflow-auto text-xs">
              <div className="grid gap-1 sm:grid-cols-2 xl:grid-cols-3 min-[1900px]:flex min-[1900px]:flex-col">
                {[...tables, ...views].map((item) => (
                  <button
                    type="button"
                    key={`${item.kind}:${item.schema}.${item.name}`}
                    aria-label={`Open ${item.kind} ${item.schema}.${item.name}`}
                    onClick={() =>
                      item.kind === "table"
                        ? onOpenTable(item.schema, item.name)
                        : onOpenView(item.schema, item.name)
                    }
                    className="flex min-w-0 items-center justify-between gap-2 rounded-md border border-transparent px-2 py-1.5 text-left transition hover:border-border hover:bg-muted/40"
                  >
                    <span className="min-w-0 truncate">
                      <span className="text-muted-foreground">
                        {item.schema}.
                      </span>
                      {item.name}
                    </span>
                    <span className="shrink-0 rounded-sm bg-muted px-1.5 py-0.5 text-[0.625rem] uppercase text-muted-foreground">
                      {item.kind}
                    </span>
                  </button>
                ))}
              </div>
            </CardContent>
          </Card>
        </section>
      </div>
    </div>
  );
}

function StatCard({ label, value }: { label: string; value: number | string }) {
  return (
    <Card className="border border-border">
      <CardContent className="flex items-center justify-between">
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className="text-lg font-semibold tabular-nums">{value}</div>
      </CardContent>
    </Card>
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
    return "...";
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
