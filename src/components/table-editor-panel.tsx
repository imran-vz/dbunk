import { useEffect, useMemo, useState } from "react";
import ReactFlow, { Background, Controls, MiniMap } from "reactflow";

import { DataGrid, type TableViewMode } from "@/components/data-grid";
import {
  type TableStructureData,
  TableStructureView,
} from "@/components/table-structure-view";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type TablePreviewData,
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
    tablePreviews,
    tableEdits,
    openQueryForTable,
    loadTablePreview,
    setTableEdit,
    discardTableEdits,
    toggleLeftSidebar,
  } = useAppStore();

  useEffect(() => {
    if (tab.kind === "table" && tab.table) {
      void loadTablePreview(tab.schema, tab.table);
    }
  }, [tab, loadTablePreview]);

  const activeTablePreview = useMemo(() => {
    if (tab.kind !== "table") {
      return null;
    }
    return (
      tablePreviews[tab.table ?? ""] ?? {
        columns: [],
        rows: [],
        rowCount: "0",
        primaryKey: "id",
        size: "0 B",
        lastVacuum: "Never",
      }
    );
  }, [tab, tablePreviews]);

  const tableName = tab.table ?? "";
  const currentEdits = tableEdits[tableName];
  const hasEdits = Object.keys(currentEdits ?? {}).length > 0;

  // Generate mock structure data based on the table preview
  const structureData: TableStructureData = useMemo(() => {
    const columns = (activeTablePreview?.columns ?? []).map((colName) => {
      // Infer some structure from column names
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
      columns,
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
  }, [activeTablePreview, tab.table, tab.schema]);

  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex-1 overflow-hidden max-w-[calc(100vw-16rem)]">
        {viewMode === "data" ? (
          <DataGrid
            data={activeTablePreview?.rows ?? []}
            columns={activeTablePreview?.columns ?? []}
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
