import { IconCopy, IconMaximize, IconX } from "@tabler/icons-react";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import type { ForeignKeyTarget } from "@/components/data-grid";
import { MutationReviewPanel } from "@/components/mutation-review";
import {
  SchemaRelationshipMap,
  type SchemaRelationshipMapHandle,
} from "@/components/schema-relationship-map";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { AddRowForm } from "@/components/table-editor/add-row-form";
import { TableEditorBody } from "@/components/table-editor/body";
import { DataImportWizard } from "@/components/table-editor/data-import-wizard";
import {
  type SubTab,
  TableEditorHeader,
} from "@/components/table-editor/header";
import { InlineDrilldown } from "@/components/table-editor/inline-drilldown";
import { SeedTableForm } from "@/components/table-editor/seed-table-form";
import { TableStatusBanners } from "@/components/table-editor/status-banners";
import { buildStatusItems } from "@/components/table-editor/status-items";
import { useRowDetailsVisibility } from "@/components/table-editor/use-row-details-visibility";
import { useRowSelection } from "@/components/table-editor/use-row-selection";
import { useTableExportFilename } from "@/components/table-editor/use-table-export-filename";
import { useTableSession } from "@/components/table-editor/use-table-session";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
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
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import { DEFAULT_SCHEMA_MAP_PREFS } from "@/lib/schema-graph";
import type {
  SeedColumnSpecPayload,
  SeedTableProgress,
  SeedTableResult,
} from "@/lib/seed-form";
import {
  type Connection,
  type EditOutcome,
  type TablePreviewData,
  tableDataKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { deriveSelectedTableSessionCapabilities } from "@/lib/table-session";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";
import { useContainerWidth } from "@/lib/use-resizable-width";

/**
 * Engines with a backend `seed_table` implementation (ADR-0020). Redis
 * is absent because Table Seeding has no meaning for a keyvalue store,
 * not because it hasn't caught up.
 */
const SEEDABLE_ENGINES = new Set([
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
]);

interface TableEditorPanelProps {
  tab: WorkspaceTab;
  variant?: "default" | "workbench";
  activeSubTab?: SubTab;
  onSubTabChange?: (next: SubTab) => void;
  onStatusItemsChange?: (items: StatusBarItem[]) => void;
}

export function TableEditorPanel({
  tab,
  variant = "default",
  activeSubTab: controlledSubTab,
  onSubTabChange,
  onStatusItemsChange,
}: TableEditorPanelProps) {
  const [internalSubTab, setInternalSubTab] = useState<SubTab>("data");
  const activeSubTab = controlledSubTab ?? internalSubTab;
  const setActiveSubTab = onSubTabChange ?? setInternalSubTab;
  const [bodyRef, bodyWidth] = useContainerWidth<HTMLDivElement>();

  const tableSession = useTableSession(tab);
  const {
    ref,
    key: dataKey,
    tableName,
    data,
    structure,
    status,
    commitStatus,
    currentEdits,
    hasEdits,
    pagination,
    serverBrowse,
    appliedBrowseRequestId,
    readOnlyCopy,
    mutationEnabled,
    mutationScope,
    mutationDraft,
    mutationAnalysis,
    mutationTable,
    mutationAnalysisState,
    mutationStatusCopy,
    mutationLocked,
    editableMutationColumns,
    insertableMutationColumns,
    virtualKeyColumns,
    needsVirtualKey,
  } = tableSession;
  // Terminal outcome lives component-local. Disappears on tab unmount,
  // which is the intended trade-off (CONTEXT.md — Edit Outcome).
  const [lastOutcome, setLastOutcome] = useState<EditOutcome | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [pendingDiscard, setPendingDiscard] = useState<(() => void) | null>(
    null,
  );
  const [exportError, setTableExportError] = useState<string | null>(null);
  const [savedExportTask, setSavedExportTask] =
    useState<SavedExportTask | null>(() =>
      findExportTask({
        connectionId: tab.connectionId,
        schema: tab.schema,
        table: tab.table ?? "",
      }),
    );

  const { openQueryForTable, openTableTab, refreshTableBrowsesForRelation } =
    useAppStore();

  const [isAddRowOpen, setIsAddRowOpen] = useState(false);
  const [addRowSource, setAddRowSource] = useState<"new" | "duplicate">("new");
  const [addRowInitialValues, setAddRowInitialValues] = useState<
    InsertRowPayloadEntry[] | undefined
  >();
  const [isBulkEditOpen, setIsBulkEditOpen] = useState(false);
  const [isVirtualKeyOpen, setIsVirtualKeyOpen] = useState(false);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [isImportOpen, setIsImportOpen] = useState(false);
  const [isCopyOpen, setIsCopyOpen] = useState(false);
  const [isSeedOpen, setIsSeedOpen] = useState(false);
  const [isSeeding, setIsSeeding] = useState(false);
  const [seedProgress, setSeedProgress] = useState<
    { rowsCompleted: number; totalRows: number } | undefined
  >();
  const [inlineDrilldown, setInlineDrilldown] = useState<{
    rowIndex: number;
    target: ForeignKeyTarget;
    value: string;
  } | null>(null);
  const rowDetails = useRowDetailsVisibility(bodyWidth);

  // Reset table-switch-scoped UI state. The panel instance is reused
  // across tab switches (no React key), so without this the inline
  // drill-down and terminal-outcome badge from table A would leak
  // into table B.
  useEffect(() => {
    setInlineDrilldown(null);
    setLastOutcome(null);
    setIsAddRowOpen(false);
    setIsBulkEditOpen(false);
    setIsVirtualKeyOpen(false);
    setReviewOpen(false);
  }, [tableName]);

  useEffect(() => {
    if (!inlineDrilldown) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setInlineDrilldown(null);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [inlineDrilldown]);

  useEffect(() => {
    if (!expanded) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setExpanded(false);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [expanded]);

  const connections = useAppStore((s) => s.connections);
  const connection = connections.find((c) => c.id === tab.connectionId);

  const rows = data?.rows ?? [];
  const selection = useRowSelection(rows);
  const isBrowseMode = Boolean(serverBrowse);
  useEffect(() => {
    if (!isBrowseMode) return;
    selection.clear();
    setInlineDrilldown(null);
    // Rows were replaced by a new browse page.
    // oxlint-disable-next-line react-hooks/exhaustive-deps -- selection.clear is stable enough; we only want row-identity changes.
  }, [isBrowseMode, tab.id, appliedBrowseRequestId]);
  const caps = deriveSelectedTableSessionCapabilities(
    tableSession.capabilities,
    selection.selectedCount,
  );
  const exportFilenameBase = useTableExportFilename(tab);

  const refreshTable = () => {
    void tableSession.refresh();
  };

  const handleSubmitAddRow = async (values: InsertRowPayloadEntry[]) => {
    if (mutationEnabled) {
      const staged = await tableSession.stageInsert(values, addRowSource);
      if (staged) {
        setIsAddRowOpen(false);
        setAddRowInitialValues(undefined);
      }
      return;
    }
    const outcome = await tableSession.addRow(values);
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
        const outcome = await tableSession.addRow(
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
    if (!connection) {
      setLastOutcome({ kind: "failed", reason: "Connection not found." });
      return;
    }
    try {
      const result = await invokeWithSafetyConfirmation<{
        runtimeMs: number;
        rowsAffected: number;
      }>({
        command: "import_rows",
        connection,
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
        await tableSession.refreshData();
      }
    } catch (error) {
      const message = errorToMessage(error);
      setLastOutcome({ kind: "failed", reason: message });
      toast.error(`Import failed: ${message}`);
    }
  };

  const handleDeleteSelected = async () => {
    if (selection.selectedCount === 0) return;
    if (mutationEnabled) {
      if (await tableSession.stageDeleteRows(selection.selectedIndices)) {
        selection.clear();
      }
      return;
    }
    const message = `Delete ${selection.selectedCount} row${
      selection.selectedCount === 1 ? "" : "s"
    }? This cannot be undone.`;
    if (!window.confirm(message)) return;
    const outcome = await tableSession.deleteRows(selection.selectedIndices);
    setLastOutcome(outcome);
    if (outcome.kind === "completed") selection.clear();
  };

  const handleOpenAddRow = async () => {
    if (mutationEnabled) {
      const analysis = await tableSession.ensureMutationAnalysis();
      if (!analysis) return;
    }
    setAddRowSource("new");
    setAddRowInitialValues(undefined);
    setIsAddRowOpen(true);
  };

  const handleDuplicateSelected = async () => {
    if (selection.selectedIndex === null) return;
    const values = await tableSession.duplicateRowValues(
      selection.selectedIndex,
    );
    if (!values) return;
    setAddRowSource("duplicate");
    setAddRowInitialValues(values);
    setIsAddRowOpen(true);
  };

  const handleDiscardEdits = () => {
    if (!mutationEnabled) {
      tableSession.discardEdits();
      return;
    }
    const count = mutationDraft?.changeOrder.length ?? 0;
    if (count === 0 || mutationLocked) return;
    if (
      !window.confirm(
        `Discard ${count} staged change${count === 1 ? "" : "s"}?`,
      )
    ) {
      return;
    }
    tableSession.discardEdits();
    setReviewOpen(false);
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
    const destinationConnection = connections.find(
      (candidate) => candidate.id === payload.destinationConnectionId,
    );
    if (!destinationConnection) {
      setLastOutcome({
        kind: "failed",
        reason: "Destination connection not found.",
      });
      return;
    }
    try {
      const result = await invokeWithSafetyConfirmation<{
        runtimeMs: number;
        rowsCopied: number;
      }>({
        command: "copy_table_rows",
        connection: destinationConnection,
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
    if (!connection) {
      setLastOutcome({ kind: "failed", reason: "Connection not found." });
      return;
    }
    try {
      const result = await invokeWithSafetyConfirmation<{ runtimeMs: number }>({
        command: "run_pg_maintenance",
        connection,
        payload: {
          connectionId: tab.connectionId,
          schema: tab.schema,
          table: tab.table ?? "",
          action,
        },
      });
      setLastOutcome({
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: 0,
      });
    } catch (error) {
      setLastOutcome({ kind: "failed", reason: errorToMessage(error) });
    }
  };

  const handleSeedTable = async (params: {
    rowCount: number;
    seed?: number;
    columns: SeedColumnSpecPayload[];
  }) => {
    if (!isTauri()) {
      setLastOutcome({
        kind: "failed",
        reason: "Table seeding requires the desktop runtime.",
      });
      return;
    }
    if (!connection) {
      setLastOutcome({ kind: "failed", reason: "Connection not found." });
      return;
    }
    setIsSeeding(true);
    setSeedProgress({ rowsCompleted: 0, totalRows: params.rowCount });
    const operationId =
      globalThis.crypto?.randomUUID?.() ??
      `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<SeedTableProgress>(
        "seed-table-progress",
        (event) => {
          if (event.payload.operationId !== operationId) return;
          setSeedProgress({
            rowsCompleted: event.payload.rowsCompleted,
            totalRows: event.payload.totalRows,
          });
        },
      ).catch(() => undefined);
      const result = await invokeWithSafetyConfirmation<SeedTableResult>({
        command: "seed_table",
        connection,
        payload: {
          operationId,
          connectionId: tab.connectionId,
          schema: tab.schema,
          table: tab.table ?? "",
          rowCount: params.rowCount,
          seed: params.seed,
          columns: params.columns,
        },
      });
      setIsSeedOpen(false);
      setLastOutcome({
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsInserted,
      });
      toast.success(
        `Seeded ${result.rowsInserted.toLocaleString()} rows (seed ${result.seedUsed})`,
      );
      void tableSession.refreshData();
    } catch (error) {
      setLastOutcome({ kind: "failed", reason: errorToMessage(error) });
      toast.error(`Seed failed: ${errorToMessage(error)}`);
    } finally {
      unlisten?.();
      setIsSeeding(false);
      setSeedProgress(undefined);
    }
  };

  const confirmIfEdits = (action: () => void) => {
    if (hasEdits && !mutationEnabled) {
      setPendingDiscard(() => action);
      return;
    }
    action();
  };

  const onRefresh = () => confirmIfEdits(refreshTable);

  const guardedBrowse = serverBrowse
    ? {
        ...serverBrowse,
        onApplyTypedFilter: (
          filter: (typeof serverBrowse)["typedFilters"][number],
        ) => confirmIfEdits(() => serverBrowse.onApplyTypedFilter(filter)),
        onRemoveTypedFilter: (column: string) =>
          confirmIfEdits(() => serverBrowse.onRemoveTypedFilter(column)),
        onClearTypedFilters: () =>
          confirmIfEdits(() => serverBrowse.onClearTypedFilters()),
        onRawFilterApply: (text: string) =>
          confirmIfEdits(() => serverBrowse.onRawFilterApply(text)),
        onSortChange: (sort: (typeof serverBrowse)["sort"]) =>
          confirmIfEdits(() => serverBrowse.onSortChange(sort)),
        onPageSizeChange: (pageSize: number) =>
          confirmIfEdits(() => serverBrowse.onPageSizeChange(pageSize)),
        onHeaderSort: (column: string, append: boolean) =>
          confirmIfEdits(() => serverBrowse.onHeaderSort(column, append)),
        onApplyPreset: (name: string) =>
          confirmIfEdits(() => serverBrowse.onApplyPreset(name)),
        onApplyHistory: (index: number) =>
          confirmIfEdits(() => serverBrowse.onApplyHistory(index)),
      }
    : undefined;

  const guardedPagination = serverBrowse
    ? {
        ...pagination,
        goToPage: (next: number) =>
          confirmIfEdits(() => pagination.goToPage(next)),
        onPrevPage: () => confirmIfEdits(() => pagination.onPrevPage()),
        onNextPage: () => confirmIfEdits(() => pagination.onNextPage()),
        onFirstPage: () => confirmIfEdits(() => pagination.onFirstPage()),
        onLastPage: () => confirmIfEdits(() => pagination.onLastPage()),
      }
    : pagination;

  const isLoading = status?.state === "loading";
  const isSaving = commitStatus?.state === "running";
  const errorMessage = status?.state === "error" ? status.error : null;

  const rowCountLabel = serverBrowse
    ? pagination.countLabel
    : `${(pagination.totalRows ?? rows.length).toLocaleString()} rows`;

  const statusItems = buildStatusItems({
    errorMessage,
    isLoading,
    rowCount: rows.length,
    rowCountLabel: serverBrowse ? pagination.countLabel : undefined,
    pagination,
    activeConnection: connection,
  });

  useEffect(() => {
    onStatusItemsChange?.(statusItems);
  }, [onStatusItemsChange, statusItems]);

  const isWorkbench = variant === "workbench";

  return (
    <div className="flex h-full flex-col bg-surface-app">
      {isLoading ? (
        <div
          data-testid="table-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}

      {!expanded ? (
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
          showSeedAction={SEEDABLE_ENGINES.has(connection?.engine ?? "")}
          onOpenSeedTable={() => setIsSeedOpen(true)}
          variant={isWorkbench ? "workbench" : "default"}
        />
      ) : null}

      <TableStatusBanners
        errorMessage={exportError ?? errorMessage}
        showReadOnlyBanner={caps.isReadOnly && caps.structureLoaded}
        readOnlyCopy={readOnlyCopy}
        commitStatus={commitStatus}
        lastOutcome={lastOutcome}
        onRetryLoad={onRefresh}
        onDismissOutcome={() => setLastOutcome(null)}
      />

      {mutationEnabled && mutationStatusCopy ? (
        <output
          data-testid="table-mutation-status"
          className="border-b border-border-subtle bg-black px-3 py-1.5 text-[0.6875rem] text-text-secondary"
        >
          {mutationStatusCopy}
        </output>
      ) : null}

      {mutationEnabled &&
      (needsVirtualKey || mutationTable?.identity.kind === "virtualKey") ? (
        <VirtualKeyEditor
          columns={data?.columns ?? []}
          savedColumns={virtualKeyColumns}
          open={isVirtualKeyOpen}
          busy={mutationAnalysisState.state === "loading"}
          onOpenChange={setIsVirtualKeyOpen}
          onSave={tableSession.saveMutationVirtualKey}
          onClear={tableSession.clearMutationVirtualKey}
        />
      ) : null}

      {isSeedOpen && structure ? (
        <SeedTableForm
          columns={structure.columns}
          isSeeding={isSeeding}
          progress={seedProgress}
          onSubmit={handleSeedTable}
          onClose={() => setIsSeedOpen(false)}
        />
      ) : null}

      {isAddRowOpen && structure ? (
        <AddRowForm
          columns={
            mutationEnabled && mutationAnalysis
              ? structure.columns.filter((column) =>
                  insertableMutationColumns.includes(column.name),
                )
              : structure.columns
          }
          isWriting={caps.isWriting}
          initialValues={addRowInitialValues}
          title={addRowSource === "duplicate" ? "Duplicate row" : "Add row"}
          submitLabel={
            addRowSource === "duplicate"
              ? "Stage duplicate"
              : mutationEnabled
                ? "Stage row"
                : "Insert"
          }
          onSubmit={handleSubmitAddRow}
          onClose={() => {
            setIsAddRowOpen(false);
            setAddRowInitialValues(undefined);
          }}
        />
      ) : null}

      {isBulkEditOpen ? (
        <BulkEditForm
          columns={editableMutationColumns}
          selectedCount={selection.selectedCount}
          isWriting={mutationLocked}
          onClose={() => setIsBulkEditOpen(false)}
          onSubmit={async (column, value) => {
            const count = await tableSession.stageBulkEdit(
              selection.selectedIndices,
              column,
              value,
            );
            if (count > 0) setIsBulkEditOpen(false);
          }}
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
        pagination={guardedPagination}
        isLoading={isLoading}
        isSaving={isSaving}
        exportFilenameBase={exportFilenameBase}
        onRefresh={onRefresh}
        onOpenAddRow={() => void handleOpenAddRow()}
        onOpenImport={() => setIsImportOpen(true)}
        onOpenSql={() => openQueryForTable(tab.schema, tab.table ?? "")}
        onOpenTable={openTableTab}
        onSubTabChange={setActiveSubTab}
        onCellEdit={(rowIndex, colIndex, value) =>
          tableSession.setCellEdit(rowIndex, colIndex, value)
        }
        onEditIntent={mutationEnabled ? tableSession.onEditIntent : undefined}
        getCellReadOnlyReason={
          mutationEnabled ? tableSession.getCellReadOnlyReason : undefined
        }
        getRowState={mutationEnabled ? tableSession.getRowState : undefined}
        onDiscardEdits={handleDiscardEdits}
        onSaveEdits={async () => {
          if (mutationEnabled) {
            setReviewOpen(true);
            return;
          }
          const outcome = await tableSession.commitEdits();
          setLastOutcome(outcome);
        }}
        onDeleteSelected={() => {
          void handleDeleteSelected();
        }}
        onDuplicateSelected={
          mutationEnabled ? () => void handleDuplicateSelected() : undefined
        }
        onBulkEditSelected={
          mutationEnabled
            ? () => {
                void tableSession.ensureMutationAnalysis().then((analysis) => {
                  if (analysis) setIsBulkEditOpen(true);
                });
              }
            : undefined
        }
        stagedChangeCount={
          mutationEnabled ? (mutationDraft?.changeOrder.length ?? 0) : 0
        }
        onOpenReview={mutationEnabled ? () => setReviewOpen(true) : undefined}
        onFollowForeignKey={(rowIndex, target, value) =>
          setInlineDrilldown((current) => {
            // Clicking the same FK arrow twice closes the expansion.
            if (
              current &&
              current.rowIndex === rowIndex &&
              current.target.schema === target.schema &&
              current.target.table === target.table &&
              current.target.column === target.column &&
              current.value === value
            ) {
              return null;
            }
            return { rowIndex, target, value };
          })
        }
        rowExpansion={
          inlineDrilldown && connection
            ? {
                rowIndex: inlineDrilldown.rowIndex,
                content: (
                  <InlineDrilldown
                    connectionId={tab.connectionId}
                    engine={connection.engine}
                    target={inlineDrilldown.target}
                    value={inlineDrilldown.value}
                    onClose={() => setInlineDrilldown(null)}
                  />
                ),
              }
            : null
        }
        onExportWholeTable={exportWholeTable}
        onSaveExportTask={handleSaveExportTask}
        onRunSavedExportTask={handleRunSavedExportTask}
        hasSavedExportTask={savedExportTask !== null}
        serverBrowse={guardedBrowse}
        onExpandGrid={() => setExpanded((open) => !open)}
        expanded={expanded}
        reviewPanel={
          mutationEnabled && reviewOpen ? (
            <div className="h-full w-[min(30rem,42vw)] shrink-0 bg-black max-[820px]:absolute max-[820px]:inset-y-0 max-[820px]:right-0 max-[820px]:z-40 max-[820px]:w-full">
              <MutationReviewPanel
                scope={mutationScope}
                onClose={() => setReviewOpen(false)}
                onApplySuccess={async () => {
                  selection.clear();
                  if (!ref) return;
                  await refreshTableBrowsesForRelation(
                    ref.connectionId,
                    ref.schema,
                    ref.table,
                    { refreshStructure: false, invalidateExactCount: true },
                  );
                }}
                onRefresh={tableSession.refreshData}
              />
            </div>
          ) : null
        }
      />

      <AlertDialog
        open={pendingDiscard !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDiscard(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Discard pending edits?</AlertDialogTitle>
            <AlertDialogDescription>
              Changing filters, sort, or page replaces the loaded rows and
              discards uncommitted cell edits.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel onClick={() => setPendingDiscard(null)}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                tableSession.discardEdits();
                pendingDiscard?.();
                setPendingDiscard(null);
              }}
            >
              Discard edits
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {!isWorkbench && !expanded ? <StatusBar items={statusItems} /> : null}
    </div>
  );
}

function VirtualKeyEditor({
  columns,
  savedColumns,
  open,
  busy,
  onOpenChange,
  onSave,
  onClear,
}: {
  columns: string[];
  savedColumns: string[];
  open: boolean;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (columns: string[]) => Promise<boolean>;
  onClear: () => Promise<boolean>;
}) {
  const [selected, setSelected] = useState(savedColumns);
  useEffect(() => setSelected(savedColumns), [savedColumns]);
  return (
    <div
      data-testid="virtual-key-editor"
      className="border-b border-border-subtle bg-black px-3 py-2 text-xs"
    >
      <div className="flex items-center gap-2">
        <span className="text-text-secondary">
          Keyless relation. Pick projected columns that uniquely identify a row.
        </span>
        <Button
          size="sm"
          variant="outline"
          className="ml-auto"
          onClick={() => onOpenChange(!open)}
        >
          {open ? "Hide virtual key" : "Choose virtual key"}
        </Button>
      </div>
      {open ? (
        <div className="mt-2 flex flex-wrap items-center gap-3">
          {columns.map((column) => (
            <label key={column} className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={selected.includes(column)}
                onChange={(event) =>
                  setSelected((current) =>
                    event.target.checked
                      ? [...current, column]
                      : current.filter((value) => value !== column),
                  )
                }
              />
              <span className="font-mono">{column}</span>
            </label>
          ))}
          <Button
            size="sm"
            disabled={busy || selected.length === 0}
            onClick={() => void onSave(selected)}
          >
            Save virtual key
          </Button>
          {savedColumns.length > 0 ? (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              onClick={() => void onClear()}
            >
              Clear virtual key
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function BulkEditForm({
  columns,
  selectedCount,
  isWriting,
  onClose,
  onSubmit,
}: {
  columns: string[];
  selectedCount: number;
  isWriting: boolean;
  onClose: () => void;
  onSubmit: (column: string, value: string | null) => Promise<void>;
}) {
  const [column, setColumn] = useState(columns[0] ?? "");
  const [value, setValue] = useState("");
  const [setNull, setSetNull] = useState(false);
  return (
    <div
      data-testid="bulk-edit-form"
      className="flex items-center gap-2 border-b border-border-subtle bg-black px-3 py-2 text-xs"
    >
      <span>Set {selectedCount} selected rows</span>
      <select
        aria-label="Bulk edit column"
        className="h-7 border border-border-subtle bg-surface-input px-2 font-mono"
        value={column}
        onChange={(event) => setColumn(event.target.value)}
      >
        {columns.map((name) => (
          <option key={name} value={name}>
            {name}
          </option>
        ))}
      </select>
      <Input
        aria-label="Bulk edit value"
        className="h-7 min-w-32 flex-1"
        value={value}
        disabled={setNull}
        onChange={(event) => setValue(event.target.value)}
      />
      <label className="flex items-center gap-1 text-text-secondary">
        <input
          type="checkbox"
          checked={setNull}
          onChange={(event) => setSetNull(event.target.checked)}
        />
        NULL
      </label>
      <Button
        size="sm"
        disabled={isWriting || !column}
        onClick={() => void onSubmit(column, setNull ? null : value)}
      >
        Stage bulk edit
      </Button>
      <Button size="sm" variant="ghost" onClick={onClose}>
        Cancel
      </Button>
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
    const key = tableDataKey(tab.connectionId, tab.schema, tab.table ?? "");
    return (
      tablePreviews[key] ?? {
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
