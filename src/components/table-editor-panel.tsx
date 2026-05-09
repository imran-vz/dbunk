import {
  IconAlertTriangle,
  IconChevronLeft,
  IconChevronRight,
  IconRefresh,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import ReactFlow, { Background, Controls, MiniMap } from "reactflow";

import { DataGrid, type TableViewMode } from "@/components/data-grid";
import {
  type TableStructureData,
  TableStructureView,
} from "@/components/table-structure-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type TablePreviewData,
  tableDataKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface TableEditorPanelProps {
  tab: WorkspaceTab;
}

export function TableEditorPanel({ tab }: TableEditorPanelProps) {
  const [viewMode, setViewMode] = useState<TableViewMode>("data");

  const {
    tableData,
    tableLoadStatus,
    tableEdits,
    openQueryForTable,
    loadTableData,
    refreshTableData,
    setTableEdit,
    discardTableEdits,
    toggleLeftSidebar,
  } = useAppStore();

  const dataKey =
    tab.kind === "table" && tab.table
      ? tableDataKey(tab.connectionId, tab.schema, tab.table)
      : "";

  useEffect(() => {
    if (tab.kind === "table" && tab.table && tab.connectionId) {
      void loadTableData(tab.connectionId, tab.schema, tab.table);
    }
  }, [tab.kind, tab.table, tab.schema, tab.connectionId, loadTableData]);

  const activeTableData = dataKey ? tableData[dataKey] : undefined;
  const tableName = tab.table ?? "";
  const status = tableName ? tableLoadStatus[tableName] : undefined;
  const currentEdits = tableEdits[tableName];
  const hasEdits = Object.keys(currentEdits ?? {}).length > 0;

  const columns = activeTableData?.columns ?? [];
  const rows = activeTableData?.rows ?? [];
  const page = activeTableData?.page ?? 1;
  const pageSize = activeTableData?.pageSize ?? 100;
  const totalRows = activeTableData?.totalRows;
  const runtimeMs = activeTableData?.runtimeMs;

  const totalPages =
    totalRows !== undefined && pageSize > 0
      ? Math.max(1, Math.ceil(totalRows / pageSize))
      : undefined;
  const isLastPage =
    totalPages !== undefined ? page >= totalPages : rows.length < pageSize;

  const onPrevPage = () => {
    if (tab.kind === "table" && tab.table && tab.connectionId && page > 1) {
      void loadTableData(
        tab.connectionId,
        tab.schema,
        tab.table,
        page - 1,
        pageSize,
      );
    }
  };

  const onNextPage = () => {
    if (tab.kind === "table" && tab.table && tab.connectionId && !isLastPage) {
      void loadTableData(
        tab.connectionId,
        tab.schema,
        tab.table,
        page + 1,
        pageSize,
      );
    }
  };

  const onRefresh = () => {
    if (dataKey) {
      void refreshTableData(dataKey);
    }
  };

  // Generate mock structure data based on the table preview
  const structureData: TableStructureData = useMemo(() => {
    const cols = columns.map((colName) => {
      const isPrimaryKey = colName === "id";
      const isTimestamp = colName.includes("_at");
      const isBoolean = colName.startsWith("is_");
      const isEmail = colName === "email";
      const isHash = colName.includes("hash") || colName.includes("password");

      let dataType = "VARCHAR(255)";
      if (isPrimaryKey) dataType = "VARCHAR(50)";
      if (isTimestamp) dataType = "TIMESTAMP WITH TIME ZONE";
      if (isBoolean) dataType = "BOOLEAN";
      if (colName === "phone") dataType = "VARCHAR(50)";

      const isNullable =
        colName === "phone" ||
        colName === "deleted_at" ||
        (!isPrimaryKey && !isEmail && !isHash && !isBoolean && !isTimestamp);

      return {
        name: colName,
        dataType,
        isPrimaryKey,
        isNullable,
        defaultValue: undefined,
        isGenerated: false,
      };
    });

    return {
      tableName: tab.table ?? "untitled",
      schema: tab.schema,
      columns: cols,
      constraints: [
        {
          name: `${tab.table}_pkey`,
          type: "PRIMARY KEY" as const,
          columns: ["id"],
        },
      ],
      indexes: [
        {
          name: `ix_${tab.table}_email`,
          isUnique: true,
          method: "BTREE",
          columns: ["email"],
        },
        {
          name: `${tab.table}_pkey`,
          isUnique: true,
          method: "BTREE",
          columns: ["id"],
        },
      ],
      policies: [],
      rowLevelSecurity: false,
    };
  }, [columns, tab.table, tab.schema]);

  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;

  const pageInfo = (() => {
    const parts: string[] = [];
    parts.push(
      totalPages !== undefined
        ? `Page ${page} of ${totalPages}`
        : `Page ${page}`,
    );
    if (totalRows !== undefined) {
      parts.push(`${totalRows.toLocaleString()} rows`);
    }
    if (runtimeMs !== undefined) {
      parts.push(`${runtimeMs} ms`);
    }
    return parts.join(" • ");
  })();

  return (
    <div className="flex h-full flex-col bg-background">
      {isLoading ? (
        <div
          data-testid="table-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}
      {errorMessage ? (
        <div
          data-testid="table-error"
          role="alert"
          className="flex items-center gap-2 border-b border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive"
        >
          <IconAlertTriangle className="size-4" />
          <span>Failed to load rows: {errorMessage}</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={onRefresh}
          >
            Retry
          </Button>
        </div>
      ) : null}
      <div className="flex-1 overflow-hidden max-w-[calc(100vw-16rem)]">
        {viewMode === "data" ? (
          <DataGrid
            data={rows}
            columns={columns}
            edits={currentEdits}
            onEdit={(rowIndex, colIndex, value) =>
              setTableEdit(tableName, rowIndex, colIndex, value)
            }
            hasEdits={hasEdits}
            onDiscard={() => discardTableEdits(tableName)}
            onSave={() => {}}
            onOpenSQL={() => openQueryForTable(tab.schema, tab.table ?? "")}
            viewMode={viewMode}
            onViewModeChange={setViewMode}
            onToggleSidebar={toggleLeftSidebar}
          />
        ) : (
          <div className="flex h-full flex-col">
            <DataGrid
              data={[]}
              columns={[]}
              viewMode={viewMode}
              onViewModeChange={setViewMode}
              onToggleSidebar={toggleLeftSidebar}
              className="h-14 flex-none"
            />
            <TableStructureView
              data={structureData}
              className="flex-1 border-t"
            />
          </div>
        )}
      </div>
      {viewMode === "data" && tab.kind === "table" ? (
        <div
          data-testid="table-pagination"
          className="flex h-9 shrink-0 items-center justify-between border-t bg-background px-4 text-xs text-muted-foreground"
        >
          <span className="tabular-nums">{pageInfo}</span>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
              onClick={onRefresh}
              aria-label="Refresh"
            >
              <IconRefresh className="mr-1 size-3.5" /> Refresh
            </Button>
            <div className="flex items-center rounded-md border bg-muted/20 p-0.5">
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 rounded-sm"
                onClick={onPrevPage}
                disabled={page <= 1 || isLoading}
                aria-label="Previous page"
              >
                <IconChevronLeft className="size-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-6 w-6 rounded-sm"
                onClick={onNextPage}
                disabled={isLastPage || isLoading}
                aria-label="Next page"
              >
                <IconChevronRight className="size-3.5" />
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export interface TableSidebarProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

export function TableSidebar({ tab, isClient }: TableSidebarProps) {
  const { activeConnectionId, tablePreviews, schemaFlows } = useAppStore();

  const activeTablePreview: TablePreviewData | null = useMemo(() => {
    if (tab.kind !== "table") {
      return null;
    }
    return (
      tablePreviews[tab.table ?? ""] ?? {
        columns: ["id", "name", "status"],
        rows: [],
        rowCount: "--",
        primaryKey: "--",
        size: "--",
        lastVacuum: "--",
      }
    );
  }, [tab, tablePreviews]);

  const flowNodes = useMemo(() => {
    const defaultFlow = { nodes: [], edges: [] };
    const flow = schemaFlows[activeConnectionId] ?? defaultFlow;
    const activeTable = tab.kind === "table" ? tab.table : null;

    return flow.nodes.map((node) => ({
      ...node,
      className: cn(
        "rounded-md border bg-card px-2 py-1 text-[0.65rem] shadow-sm",
        node.id === activeTable
          ? "border-primary bg-primary/10 text-primary"
          : "border-border text-foreground",
      ),
    }));
  }, [activeConnectionId, tab.kind, tab.table, schemaFlows]);

  const flowEdges = useMemo(() => {
    const defaultFlow = { nodes: [], edges: [] };
    return (schemaFlows[activeConnectionId] ?? defaultFlow).edges;
  }, [activeConnectionId, schemaFlows]);

  return (
    <>
      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Table insights</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="flex items-center justify-between">
            <span>Primary key</span>
            <span className="text-foreground">
              {activeTablePreview?.primaryKey ?? "--"}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span>Rows</span>
            <span className="text-foreground">
              {activeTablePreview?.rowCount ?? "--"}
            </span>
          </div>
          <div className="flex items-center justify-between">
            <span>Last vacuum</span>
            <span className="text-foreground">
              {activeTablePreview?.lastVacuum ?? "--"}
            </span>
          </div>
        </CardContent>
      </Card>

      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Columns</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs">
          {(activeTablePreview?.columns ?? []).map((column) => (
            <div
              key={column}
              className="flex items-center justify-between rounded-md border px-2 py-1"
            >
              <span className="text-muted-foreground">{column}</span>
              <Badge variant="secondary" className="text-[0.625rem]">
                text
              </Badge>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Schema map</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="h-56 overflow-hidden rounded-md border">
            {isClient ? (
              <ReactFlow
                nodes={flowNodes}
                edges={flowEdges}
                fitView
                nodesDraggable={false}
                nodesConnectable={false}
                zoomOnScroll={false}
                zoomOnDoubleClick={false}
                panOnDrag={false}
              >
                <Background gap={16} size={0.5} />
                <MiniMap pannable zoomable />
                <Controls showInteractive={false} />
              </ReactFlow>
            ) : (
              <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
                Loading schema map...
              </div>
            )}
          </div>
        </CardContent>
      </Card>

      <Card size="sm" className="border border-border">
        <CardHeader>
          <CardTitle>Access</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-muted-foreground">
          <div className="flex items-center justify-between">
            <span>Role</span>
            <span className="text-foreground">Analyst</span>
          </div>
          <div className="flex items-center justify-between">
            <span>Policy</span>
            <span className="text-foreground">Row-level</span>
          </div>
          <div className="flex items-center justify-between">
            <span>Masking</span>
            <span className="text-foreground">Enabled</span>
          </div>
        </CardContent>
      </Card>
    </>
  );
}
