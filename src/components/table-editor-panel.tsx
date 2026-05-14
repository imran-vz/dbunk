import { IconCopy, IconMaximize, IconX } from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import {
  SchemaRelationshipMap,
  type SchemaRelationshipMapHandle,
} from "@/components/schema-relationship-map";
import { StatusBar } from "@/components/status-bar";
import { AddRowForm } from "@/components/table-editor/add-row-form";
import { TableEditorBody } from "@/components/table-editor/body";
import { DataImportWizard } from "@/components/table-editor/data-import-wizard";
import {
  type SubTab,
  TableEditorHeader,
} from "@/components/table-editor/header";
import { TableStatusBanners } from "@/components/table-editor/status-banners";
import { buildStatusItems } from "@/components/table-editor/status-items";
import { useRowDetailsVisibility } from "@/components/table-editor/use-row-details-visibility";
import { useRowSelection } from "@/components/table-editor/use-row-selection";
import { useTableCapabilities } from "@/components/table-editor/use-table-capabilities";
import { useTableEditorData } from "@/components/table-editor/use-table-editor-data";
import { useTableExportFilename } from "@/components/table-editor/use-table-export-filename";
import { useTablePagination } from "@/components/table-editor/use-table-pagination";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  type SchemaMapExportFormat,
  SchemaMapToolbar,
  schemaMapExportFilename,
} from "@/components/workspace-overview/schema-map-toolbar";
import { downloadBlob, downloadFile } from "@/lib/download";
import {
  type ExportCompression,
  type ExportEncoding,
  type ExportFormat,
  type ExportTable,
  prepareExportBlob,
} from "@/lib/export";
import {
  createExportTask,
  findExportTask,
  type SavedExportTask,
  saveExportTask,
} from "@/lib/export-tasks";
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import { DEFAULT_SCHEMA_MAP_PREFS } from "@/lib/schema-graph";
import {
  type Connection,
  type EditOutcome,
  type TablePreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";
import { useContainerWidth } from "@/lib/use-resizable-width";

interface TableEditorPanelProps {
  tab: WorkspaceTab;
}

export function TableEditorPanel({ tab }: TableEditorPanelProps) {
  const [activeSubTab, setActiveSubTab] = useState<SubTab>("data");
  const [bodyRef, bodyWidth] = useContainerWidth<HTMLDivElement>();

  const editor = useTableEditorData(tab);
  const { ref, tableName, dataKey, data, structure, status } = editor;
  // Sliced subscription: re-render only when *this* table's lifecycle
  // slot changes, not when any other table's status moves.
  const commitStatus = useAppStore((s) => s.tableEditsCommitStatus[tableName]);
  // Terminal outcome lives component-local. Disappears on tab unmount,
  // which is the intended trade-off (CONTEXT.md — Edit Outcome).
  const [lastOutcome, setLastOutcome] = useState<EditOutcome | null>(null);
  const [exportError, setTableExportError] = useState<string | null>(null);
  const [savedExportTask, setSavedExportTask] =
    useState<SavedExportTask | null>(() =>
      findExportTask({
        connectionId: tab.connectionId,
        schema: tab.schema,
        table: tab.table ?? "",
      }),
    );

  const {
    tableEdits,
    openQueryForTable,
    openTableTab,
    loadTableData,
    refreshTableData,
    setTableEdit,
    discardTableEdits,
    commitTableEdits,
    addTableRow,
    deleteSelectedTableRows,
  } = useAppStore();

  const [isAddRowOpen, setIsAddRowOpen] = useState(false);
  const [isImportOpen, setIsImportOpen] = useState(false);
  const [isCopyOpen, setIsCopyOpen] = useState(false);
  const rowDetails = useRowDetailsVisibility(bodyWidth);

  // Reset the terminal-outcome badge on table switch — the panel instance
  // is reused across tab switches (no React key), so without this the
  // badge from table A could leak into table B's view.
  // biome-ignore lint/correctness/useExhaustiveDependencies: tableName is the change trigger, not a value read inside
  useEffect(() => {
    setLastOutcome(null);
  }, [tableName]);

  const currentEdits = tableEdits[tableName];
  const hasEdits = Object.keys(currentEdits ?? {}).length > 0;
  const connections = useAppStore((s) => s.connections);
  const connection = connections.find((c) => c.id === tab.connectionId);

  const rows = data?.rows ?? [];
  const selection = useRowSelection(rows);
  const caps = useTableCapabilities({
    structure,
    commitStatus,
    selectedCount: selection.selectedCount,
  });
  const exportFilenameBase = useTableExportFilename(tab);

  const pagination = useTablePagination({
    tab,
    data,
    loadTableData,
  });

  const onRefresh = () => {
    if (dataKey) void refreshTableData(dataKey);
  };

  const handleSubmitAddRow = async (values: InsertRowPayloadEntry[]) => {
    const outcome = await addTableRow(tableName, values);
    setLastOutcome(outcome);
    if (outcome.kind === "completed") {
      setIsAddRowOpen(false);
    }
  };

  const handleImportRows = async (payload: {
    columns: string[];
    rows: Array<Array<string | null>>;
    useCopy: boolean;
  }) => {
    if (!isTauri()) {
      for (const row of payload.rows) {
        const outcome = await addTableRow(
          tableName,
          payload.columns.map((column, index) => ({
            column,
            value: row[index] ?? null,
          })),
        );
        if (outcome.kind !== "completed") {
          setLastOutcome(outcome);
          return;
        }
      }
      setLastOutcome({
        kind: "completed",
        runtimeMs: 0,
        rowsAffected: payload.rows.length,
      });
      setIsImportOpen(false);
      return;
    }
    try {
      const result = await tauriInvoke<{
        runtimeMs: number;
        rowsAffected: number;
      }>("import_rows", {
        payload: {
          connectionId: tab.connectionId,
          schema: tab.schema,
          table: tab.table ?? "",
          columns: payload.columns,
          rows: payload.rows,
          useCopy: payload.useCopy,
        },
      });
      setLastOutcome({
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsAffected,
      });
      setIsImportOpen(false);
      toast.success(
        `Imported ${result.rowsAffected.toLocaleString()} row${
          result.rowsAffected === 1 ? "" : "s"
        } in ${result.runtimeMs} ms`,
      );
      if (dataKey) {
        await refreshTableData(dataKey);
      }
    } catch (error) {
      const message = errorToMessage(error);
      setLastOutcome({ kind: "failed", reason: message });
      toast.error(`Import failed: ${message}`);
    }
  };

  const handleDeleteSelected = async () => {
    if (selection.selectedCount === 0) return;
    const message = `Delete ${selection.selectedCount} row${
      selection.selectedCount === 1 ? "" : "s"
    }? This cannot be undone.`;
    if (!window.confirm(message)) return;
    const outcome = await deleteSelectedTableRows(
      tableName,
      selection.selectedIndices,
    );
    setLastOutcome(outcome);
    if (outcome.kind === "completed") selection.clear();
  };

  const exportWholeTable = async (options: {
    format: ExportFormat;
    encoding: ExportEncoding;
    compression: ExportCompression;
    nullAs: string;
  }) => {
    setTableExportError(null);
    try {
      const table = await loadWholeTableForExport({
        connectionId: tab.connectionId,
        schema: tab.schema,
        table: tab.table ?? "",
        fallback: data
          ? { columns: data.columns, rows: data.rows }
          : { columns: [], rows: [] },
      });
      const { filename, blob } = await prepareExportBlob(table, {
        format: options.format,
        filenameBase: `${exportFilenameBase}-whole-table`,
        encoding: options.encoding,
        compression: options.compression,
        nullAs: options.nullAs,
        sqlTableName: `${tab.schema}.${tab.table ?? tab.label}`,
      });
      downloadBlob(filename, blob);
      toast.success(`Exported ${tab.table ?? tab.label} as ${options.format}`);
    } catch (error) {
      const message = errorToMessage(error);
      setTableExportError(message);
      toast.error(`Export failed: ${message}`);
    }
  };

  const handleSaveExportTask = (options: {
    format: ExportFormat;
    encoding: ExportEncoding;
    compression: ExportCompression;
    nullAs: string;
  }) => {
    const task = createExportTask(
      {
        connectionId: tab.connectionId,
        schema: tab.schema,
        table: tab.table ?? "",
      },
      options.format,
      options.encoding,
      options.compression,
      options.nullAs,
    );
    saveExportTask(task);
    setSavedExportTask(task);
    setTableExportError(null);
    toast.success(`Saved export task (${options.format})`);
  };

  const handleRunSavedExportTask = async () => {
    if (!savedExportTask) {
      return;
    }
    await exportWholeTable(savedExportTask);
  };

  const handleExportTableDdl = async () => {
    if (!isTauri()) {
      setTableExportError("DDL export requires the desktop runtime.");
      return;
    }
    try {
      const result = await tauriInvoke<{ sql: string; runtimeMs: number }>(
        "export_ddl",
        {
          payload: {
            connectionId: tab.connectionId,
            scope: "table",
            schema: tab.schema,
            table: tab.table ?? "",
          },
        },
      );
      downloadFile(
        `${exportFilenameBase}.ddl.sql`,
        "application/sql;charset=utf-8",
        result.sql,
      );
      setTableExportError(null);
      toast.success("Exported DDL");
    } catch (error) {
      const message = errorToMessage(error);
      setTableExportError(message);
      toast.error(`DDL export failed: ${message}`);
    }
  };

  const handleCopyTable = async (payload: {
    destinationConnectionId: string;
    destinationSchema: string;
    destinationTable: string;
  }) => {
    if (!isTauri()) {
      setLastOutcome({
        kind: "failed",
        reason: "Table copy requires the desktop runtime.",
      });
      return;
    }
    try {
      const result = await tauriInvoke<{
        runtimeMs: number;
        rowsCopied: number;
      }>("copy_table_rows", {
        payload: {
          sourceConnectionId: tab.connectionId,
          sourceSchema: tab.schema,
          sourceTable: tab.table ?? "",
          destinationConnectionId: payload.destinationConnectionId,
          destinationSchema: payload.destinationSchema,
          destinationTable: payload.destinationTable,
          pageSize: 1000,
        },
      });
      setLastOutcome({
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsCopied,
      });
      setIsCopyOpen(false);
    } catch (error) {
      setLastOutcome({ kind: "failed", reason: errorToMessage(error) });
    }
  };

  const handleRunMaintenance = async (
    action: "vacuum" | "analyze" | "reindex",
  ) => {
    if (!isTauri()) {
      setLastOutcome({
        kind: "failed",
        reason: "Table maintenance requires the desktop runtime.",
      });
      return;
    }
    try {
      const result = await tauriInvoke<{ runtimeMs: number }>(
        "run_pg_maintenance",
        {
          payload: {
            connectionId: tab.connectionId,
            schema: tab.schema,
            table: tab.table ?? "",
            action,
          },
        },
      );
      setLastOutcome({
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: 0,
      });
    } catch (error) {
      setLastOutcome({ kind: "failed", reason: errorToMessage(error) });
    }
  };

  const isLoading = status?.state === "loading";
  const isSaving = commitStatus?.state === "running";
  const errorMessage = status?.state === "error" ? status.error : null;

  const rowCountLabel = `${(pagination.totalRows ?? rows.length).toLocaleString()} rows`;

  const statusItems = buildStatusItems({
    errorMessage,
    isLoading,
    rowCount: rows.length,
    pagination,
    connectionStatus: connection?.status,
  });

  return (
    <div className="flex h-full flex-col bg-surface-app">
      {isLoading ? (
        <div
          data-testid="table-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}

      <TableEditorHeader
        title={tab.table ?? tab.label}
        schemaBadge={tab.schema}
        rowCountLabel={rowCountLabel}
        activeSubTab={activeSubTab}
        onSubTabChange={setActiveSubTab}
        showRowDetailsToggle={activeSubTab === "data"}
        rowDetailsVisible={rowDetails.visible}
        onToggleRowDetails={rowDetails.onToggle}
        onOpenSql={() => openQueryForTable(tab.schema, tab.table ?? "")}
        onRefresh={onRefresh}
        onExportTableDdl={handleExportTableDdl}
        onOpenCopyTable={() => setIsCopyOpen(true)}
        onRunMaintenance={handleRunMaintenance}
      />

      <TableStatusBanners
        errorMessage={exportError ?? errorMessage}
        showReadOnlyBanner={caps.isReadOnly && caps.structureLoaded}
        commitStatus={commitStatus}
        lastOutcome={lastOutcome}
        onRetryLoad={onRefresh}
        onDismissOutcome={() => setLastOutcome(null)}
      />

      {isAddRowOpen && structure ? (
        <AddRowForm
          columns={structure.columns}
          isWriting={caps.isWriting}
          onSubmit={handleSubmitAddRow}
          onClose={() => setIsAddRowOpen(false)}
        />
      ) : null}

      {isImportOpen && structure && connection ? (
        <DataImportWizard
          columns={structure.columns}
          engine={connection.engine}
          isWriting={caps.isWriting}
          onClose={() => setIsImportOpen(false)}
          onImportRows={handleImportRows}
        />
      ) : null}

      {isCopyOpen ? (
        <CopyTablePanel
          connections={connections}
          currentConnectionId={tab.connectionId}
          defaultSchema={tab.schema}
          defaultTable={tab.table ?? ""}
          isWriting={caps.isWriting}
          onClose={() => setIsCopyOpen(false)}
          onSubmit={handleCopyTable}
        />
      ) : null}

      <TableEditorBody
        bodyRef={bodyRef}
        bodyWidth={bodyWidth}
        activeSubTab={activeSubTab}
        tableRef={ref}
        schema={tab.schema}
        connectionId={tab.connectionId}
        tableName={tab.table ?? ""}
        data={data}
        structure={structure}
        currentEdits={currentEdits}
        hasEdits={hasEdits}
        selection={selection}
        caps={caps}
        rowDetails={rowDetails}
        pagination={pagination}
        isLoading={isLoading}
        isSaving={isSaving}
        exportFilenameBase={exportFilenameBase}
        onRefresh={onRefresh}
        onOpenAddRow={() => setIsAddRowOpen(true)}
        onOpenImport={() => setIsImportOpen(true)}
        onOpenSql={() => openQueryForTable(tab.schema, tab.table ?? "")}
        onOpenTable={openTableTab}
        onSubTabChange={setActiveSubTab}
        onCellEdit={(rowIndex, colIndex, value) =>
          setTableEdit(tableName, rowIndex, colIndex, value)
        }
        onDiscardEdits={() => discardTableEdits(tableName)}
        onSaveEdits={async () => {
          const outcome = await commitTableEdits(tableName);
          setLastOutcome(outcome);
        }}
        onDeleteSelected={() => {
          void handleDeleteSelected();
        }}
        onExportWholeTable={exportWholeTable}
        onSaveExportTask={handleSaveExportTask}
        onRunSavedExportTask={handleRunSavedExportTask}
        hasSavedExportTask={savedExportTask !== null}
      />

      <StatusBar items={statusItems} />
    </div>
  );
}

function CopyTablePanel({
  connections,
  currentConnectionId,
  defaultSchema,
  defaultTable,
  isWriting,
  onClose,
  onSubmit,
}: {
  connections: Connection[];
  currentConnectionId: string;
  defaultSchema: string;
  defaultTable: string;
  isWriting: boolean;
  onClose: () => void;
  onSubmit: (payload: {
    destinationConnectionId: string;
    destinationSchema: string;
    destinationTable: string;
  }) => Promise<void>;
}) {
  const relationalConnections = connections.filter(
    (connection) => connection.engine !== "Redis",
  );
  const [destinationConnectionId, setDestinationConnectionId] =
    useState(currentConnectionId);
  const [destinationSchema, setDestinationSchema] = useState(defaultSchema);
  const [destinationTable, setDestinationTable] = useState(defaultTable);

  return (
    <div className="border-b border-border-subtle bg-surface-window px-3 py-3">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-semibold">Copy table rows</div>
          <div className="text-xs text-text-muted">
            Source columns are copied into matching destination columns
          </div>
        </div>
        <Button type="button" variant="ghost" size="icon-sm" onClick={onClose}>
          <IconX className="size-3.5" />
        </Button>
      </div>
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_12rem_12rem]">
        <select
          aria-label="Destination connection"
          className="h-8 rounded-sm border border-border-subtle bg-surface-input px-2 text-xs"
          value={destinationConnectionId}
          onChange={(event) => setDestinationConnectionId(event.target.value)}
        >
          {relationalConnections.map((connection) => (
            <option key={connection.id} value={connection.id}>
              {connection.name}
            </option>
          ))}
        </select>
        <Input
          aria-label="Destination schema"
          value={destinationSchema}
          onChange={(event) => setDestinationSchema(event.target.value)}
          placeholder="Schema"
        />
        <Input
          aria-label="Destination table"
          value={destinationTable}
          onChange={(event) => setDestinationTable(event.target.value)}
          placeholder="Table"
        />
      </div>
      <div className="mt-3 flex justify-end">
        <Button
          type="button"
          disabled={
            isWriting ||
            destinationConnectionId.length === 0 ||
            destinationSchema.trim().length === 0 ||
            destinationTable.trim().length === 0
          }
          onClick={() =>
            void onSubmit({
              destinationConnectionId,
              destinationSchema: destinationSchema.trim(),
              destinationTable: destinationTable.trim(),
            })
          }
        >
          <IconCopy className="size-3.5" />
          Copy rows
        </Button>
      </div>
    </div>
  );
}

type TableDataResult = {
  columns: string[];
  rows: string[][];
  page: number;
  pageSize: number;
  totalRows?: number | null;
};

async function loadWholeTableForExport(params: {
  connectionId: string;
  schema: string;
  table: string;
  fallback: ExportTable;
}): Promise<ExportTable> {
  if (!isTauri()) {
    return params.fallback;
  }
  const pageSize = 1000;
  let page = 1;
  let columns: string[] = [];
  const rows: string[][] = [];
  while (true) {
    const result = await tauriInvoke<TableDataResult>("load_table_data", {
      payload: {
        connectionId: params.connectionId,
        schema: params.schema,
        table: params.table,
        page,
        pageSize,
      },
    });
    if (columns.length === 0) {
      columns = result.columns;
    }
    rows.push(...result.rows);
    const totalRows = result.totalRows ?? null;
    if (
      result.rows.length === 0 ||
      result.rows.length < pageSize ||
      (totalRows !== null && rows.length >= totalRows)
    ) {
      break;
    }
    page += 1;
  }
  return { columns, rows };
}

export interface TableSidebarProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

export function TableSidebar({ tab, isClient }: TableSidebarProps) {
  const {
    connectionSchemaMapSchema,
    connections,
    resetSchemaMapPositions,
    schemaExplorer,
    schemaMapPrefs,
    setConnectionSchemaMapSchema,
    setSchemaMapPref,
    tablePreviews,
  } = useAppStore();
  const [isSchemaMapFullscreen, setIsSchemaMapFullscreen] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  const fullscreenMapRef = useRef<SchemaRelationshipMapHandle>(null);

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

  const activeTable = tab.kind === "table" ? (tab.table ?? null) : null;
  const connection = connections.find((entry) => entry.id === tab.connectionId);
  const schemaNames = useMemo(() => {
    const names =
      schemaExplorer[tab.connectionId]?.map((schema) => schema.name) ?? [];
    const unique = [...new Set([tab.schema, ...names].filter(Boolean))];
    return unique.sort();
  }, [schemaExplorer, tab.connectionId, tab.schema]);
  const fullscreenSchema =
    connectionSchemaMapSchema[tab.connectionId] ?? tab.schema;
  const fullscreenPrefs =
    schemaMapPrefs[tab.connectionId]?.[fullscreenSchema] ??
    DEFAULT_SCHEMA_MAP_PREFS;

  useEffect(() => {
    if (!isSchemaMapFullscreen) {
      return;
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setIsSchemaMapFullscreen(false);
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [isSchemaMapFullscreen]);

  const handleFullscreenExport = async (format: SchemaMapExportFormat) => {
    setExportError(null);
    try {
      await fullscreenMapRef.current?.exportImage(
        format,
        schemaMapExportFilename(
          connection?.name ?? tab.connectionId,
          fullscreenSchema,
          format,
        ),
      );
    } catch (error) {
      setExportError(error instanceof Error ? error.message : String(error));
    }
  };

  return (
    <>
      <Card size="sm">
        <CardHeader>
          <CardTitle>Table insights</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-text-muted">
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

      <Card size="sm">
        <CardHeader>
          <CardTitle>Columns</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs">
          {(activeTablePreview?.columns ?? []).map((column) => (
            <div
              key={column}
              className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-app px-2 py-1"
            >
              <span className="text-text-muted">{column}</span>
              <Badge variant="secondary" className="text-[0.625rem]">
                text
              </Badge>
            </div>
          ))}
        </CardContent>
      </Card>

      <Card size="sm">
        <CardHeader>
          <CardTitle>Schema map</CardTitle>
          <CardAction>
            <Button
              type="button"
              size="icon-sm"
              variant="ghost"
              aria-label="Open schema map fullscreen"
              onClick={() => setIsSchemaMapFullscreen(true)}
            >
              <IconMaximize className="size-3.5" />
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent>
          <div className="h-56 overflow-hidden rounded-md border border-border-subtle bg-surface-app">
            <SchemaRelationshipMap
              connectionId={tab.connectionId}
              schema={tab.schema}
              activeTable={activeTable}
              isClient={isClient}
            />
          </div>
        </CardContent>
      </Card>

      {isSchemaMapFullscreen ? (
        <div
          data-testid="schema-map-fullscreen"
          className="fixed inset-0 z-50 flex flex-col bg-surface-app"
        >
          <div className="flex h-10 shrink-0 items-center justify-between border-b border-border-subtle bg-surface-window px-3">
            <div className="min-w-0">
              <div className="text-sm font-semibold">Schema map</div>
              <div className="truncate text-xs text-text-muted">
                {fullscreenSchema}
                {activeTable && fullscreenSchema === tab.schema
                  ? ` / ${activeTable}`
                  : ""}
              </div>
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => setIsSchemaMapFullscreen(false)}
            >
              <IconX className="size-3.5" />
              Close
            </Button>
          </div>
          <SchemaMapToolbar
            schemas={schemaNames}
            selectedSchema={fullscreenSchema}
            prefs={fullscreenPrefs}
            exportError={exportError}
            onSchemaChange={(schema) =>
              setConnectionSchemaMapSchema(tab.connectionId, schema)
            }
            onPrefsChange={(patch) => {
              void setSchemaMapPref(tab.connectionId, fullscreenSchema, patch);
            }}
            onResetLayout={() => {
              void resetSchemaMapPositions(tab.connectionId, fullscreenSchema);
            }}
            onExport={(format) => {
              void handleFullscreenExport(format);
            }}
          />
          <div className="min-h-0 flex-1 p-3">
            <div className="h-full overflow-hidden rounded-md border border-border-subtle bg-surface-window">
              <SchemaRelationshipMap
                ref={fullscreenMapRef}
                connectionId={tab.connectionId}
                schema={fullscreenSchema}
                activeTable={
                  fullscreenSchema === tab.schema ? activeTable : null
                }
                isClient={isClient}
              />
            </div>
          </div>
        </div>
      ) : null}

      <Card size="sm">
        <CardHeader>
          <CardTitle>Access</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2 text-xs text-text-muted">
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
