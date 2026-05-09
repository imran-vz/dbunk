import {
  IconAlertTriangle,
  IconCheck,
  IconChevronLeft,
  IconChevronRight,
  IconLock,
  IconPlus,
  IconRefresh,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import type { RowSelectionState } from "@tanstack/react-table";
import { useEffect, useMemo, useState } from "react";

import { DataGrid, type TableViewMode } from "@/components/data-grid";
import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { TableStructureView } from "@/components/table-structure-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  buildInsertValuesPayload,
  type InsertRowFieldMode,
  type InsertRowFormState,
  initialFormState,
} from "@/lib/insert-row-form";
import { pickRowIdentity } from "@/lib/row-identity";
import {
  type TablePreviewData,
  tableDataKey,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

interface TableEditorPanelProps {
  tab: WorkspaceTab;
}

export function TableEditorPanel({ tab }: TableEditorPanelProps) {
  const [viewMode, setViewMode] = useState<TableViewMode>("data");

  const {
    tableData,
    tableStructure,
    tableLoadStatus,
    tableEdits,
    tableEditsCommitStatus,
    openQueryForTable,
    loadTableData,
    loadTableStructure,
    refreshTableData,
    setTableEdit,
    discardTableEdits,
    commitTableEdits,
    clearTableEditsCommitStatus,
    addTableRow,
    deleteSelectedTableRows,
    toggleLeftSidebar,
  } = useAppStore();

  // Row selection lives here so the Delete Selected action can read the
  // selected indices and clear them after a successful delete.
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});
  const [isAddRowOpen, setIsAddRowOpen] = useState(false);
  const [addRowForm, setAddRowForm] = useState<InsertRowFormState>({});

  const dataKey =
    tab.kind === "table" && tab.table
      ? tableDataKey(tab.connectionId, tab.schema, tab.table)
      : "";
  const structureKey =
    tab.kind === "table" && tab.table
      ? tableStructureKey(tab.connectionId, tab.schema, tab.table)
      : "";

  useEffect(() => {
    if (tab.kind === "table" && tab.table && tab.connectionId) {
      void loadTableData(tab.connectionId, tab.schema, tab.table);
      // Structure is needed to discover identity columns for safe edits.
      void loadTableStructure(tab.connectionId, tab.schema, tab.table);
    }
  }, [
    tab.kind,
    tab.table,
    tab.schema,
    tab.connectionId,
    loadTableData,
    loadTableStructure,
  ]);

  const activeTableData = dataKey ? tableData[dataKey] : undefined;
  const activeTableStructure = structureKey
    ? tableStructure[structureKey]
    : undefined;
  const tableName = tab.table ?? "";
  const status = tableName ? tableLoadStatus[tableName] : undefined;
  const currentEdits = tableEdits[tableName];
  const hasEdits = Object.keys(currentEdits ?? {}).length > 0;
  const rowIdentity = pickRowIdentity(activeTableStructure);
  const isReadOnly = rowIdentity === null;
  const commitStatus = tableEditsCommitStatus[tableName];
  const connections = useAppStore((s) => s.connections);
  const connection = connections.find((c) => c.id === tab.connectionId);
  const isPostgres = connection?.engine === "PostgreSQL";
  const structureLoaded = Boolean(activeTableStructure);
  const isWriting = commitStatus?.state === "running";
  const selectedRowIndices = useMemo(
    () =>
      Object.entries(rowSelection)
        .filter(([, selected]) => selected)
        .map(([rowId]) => Number.parseInt(rowId, 10))
        .filter((n) => Number.isFinite(n)),
    [rowSelection],
  );
  const canDeleteSelected =
    selectedRowIndices.length > 0 && isPostgres && !isReadOnly && !isWriting;
  const canAddRow = structureLoaded && isPostgres && !isWriting;

  const columns = activeTableData?.columns ?? [];
  const rows = activeTableData?.rows ?? [];
  const exportFilenameBase = useMemo(() => {
    if (tab.kind !== "table" || !tab.table) {
      return "export";
    }
    const today = new Date().toISOString().slice(0, 10);
    const slug = (value: string) =>
      value
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/^-+|-+$/g, "");
    return [tab.connectionId, tab.schema, tab.table, today]
      .map(slug)
      .filter(Boolean)
      .join("-");
  }, [tab.kind, tab.connectionId, tab.schema, tab.table]);
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

  const handleOpenAddRow = () => {
    if (!activeTableStructure) {
      return;
    }
    setAddRowForm(initialFormState(activeTableStructure.columns));
    setIsAddRowOpen(true);
  };

  const handleCloseAddRow = () => {
    setIsAddRowOpen(false);
  };

  const handleSetAddRowMode = (column: string, mode: InsertRowFieldMode) => {
    setAddRowForm((prev) => ({
      ...prev,
      [column]: { mode, value: prev[column]?.value ?? "" },
    }));
  };

  const handleSetAddRowValue = (column: string, value: string) => {
    setAddRowForm((prev) => ({
      ...prev,
      [column]: { mode: prev[column]?.mode ?? "value", value },
    }));
  };

  const handleSubmitAddRow = async () => {
    if (!activeTableStructure) {
      return;
    }
    const values = buildInsertValuesPayload(
      addRowForm,
      activeTableStructure.columns,
    );
    const before = useAppStore.getState().tableEditsCommitStatus[tableName];
    await addTableRow(tableName, values);
    const after = useAppStore.getState().tableEditsCommitStatus[tableName];
    // Only close the form when the call landed in `success`. On error keep
    // the form open so the user can correct values without re-typing.
    if (after?.state === "success" && after !== before) {
      setIsAddRowOpen(false);
    }
  };

  const handleDeleteSelected = async () => {
    if (selectedRowIndices.length === 0) {
      return;
    }
    const message = `Delete ${selectedRowIndices.length} row${
      selectedRowIndices.length === 1 ? "" : "s"
    }? This cannot be undone.`;
    if (!window.confirm(message)) {
      return;
    }
    const before = useAppStore.getState().tableEditsCommitStatus[tableName];
    await deleteSelectedTableRows(tableName, selectedRowIndices);
    const after = useAppStore.getState().tableEditsCommitStatus[tableName];
    if (after?.state === "success" && after !== before) {
      // Clear selection only on success — preserves selection so the user
      // can retry or inspect what failed.
      setRowSelection({});
    }
  };

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
      {isReadOnly && activeTableStructure ? (
        <output
          data-testid="table-readonly-banner"
          className="flex items-center gap-2 border-b border-amber-500/40 bg-amber-500/10 px-4 py-2 text-xs text-amber-700 dark:text-amber-400"
        >
          <IconLock className="size-4" />
          <span>
            This table has no primary key or non-null unique index — it is
            read-only. Add a unique constraint to enable editing.
          </span>
        </output>
      ) : null}
      {commitStatus?.state === "success" ? (
        <output
          data-testid="table-commit-success"
          className="flex items-center gap-2 border-b border-emerald-500/40 bg-emerald-500/10 px-4 py-2 text-xs text-emerald-700 dark:text-emerald-400"
        >
          <IconCheck className="size-4" />
          <span>
            Saved {commitStatus.rowsAffected} row
            {commitStatus.rowsAffected === 1 ? "" : "s"} in{" "}
            {commitStatus.runtimeMs} ms.
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() => clearTableEditsCommitStatus(tableName)}
          >
            Dismiss
          </Button>
        </output>
      ) : null}
      {commitStatus?.state === "error" ? (
        <div
          data-testid="table-commit-error"
          role="alert"
          className="flex items-center gap-2 border-b border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive"
        >
          <IconX className="size-4" />
          <span>Failed to save: {commitStatus.error}</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() => clearTableEditsCommitStatus(tableName)}
          >
            Dismiss
          </Button>
        </div>
      ) : null}
      {isAddRowOpen && activeTableStructure ? (
        <div
          data-testid="add-row-form"
          className="flex flex-col gap-2 border-b bg-muted/10 px-4 py-3"
        >
          <div className="flex items-center justify-between">
            <span className="text-xs font-semibold">Add row</span>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
              onClick={handleCloseAddRow}
            >
              Cancel
            </Button>
          </div>
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {activeTableStructure.columns.map((column) => {
              const field = addRowForm[column.name] ?? {
                mode: "value" as InsertRowFieldMode,
                value: "",
              };
              const hasDefault =
                column.defaultValue !== null &&
                column.defaultValue !== undefined;
              return (
                <div
                  key={column.name}
                  className="flex flex-col gap-1 rounded-md border bg-background px-2 py-1.5"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-medium">{column.name}</span>
                    <span className="text-[0.625rem] text-muted-foreground">
                      {column.dataType}
                    </span>
                  </div>
                  <Input
                    data-testid={`add-row-value-${column.name}`}
                    className="h-7 text-xs"
                    value={field.value}
                    placeholder={
                      hasDefault ? `default: ${column.defaultValue}` : ""
                    }
                    disabled={field.mode !== "value"}
                    onChange={(e) =>
                      handleSetAddRowValue(column.name, e.target.value)
                    }
                  />
                  <div className="flex items-center gap-3 text-[0.625rem] text-muted-foreground">
                    <label className="flex items-center gap-1">
                      <input
                        type="radio"
                        name={`add-row-mode-${column.name}`}
                        data-testid={`add-row-mode-value-${column.name}`}
                        checked={field.mode === "value"}
                        onChange={() =>
                          handleSetAddRowMode(column.name, "value")
                        }
                      />
                      value
                    </label>
                    <label className="flex items-center gap-1">
                      <input
                        type="radio"
                        name={`add-row-mode-${column.name}`}
                        data-testid={`add-row-mode-null-${column.name}`}
                        disabled={!column.nullable}
                        checked={field.mode === "null"}
                        onChange={() =>
                          handleSetAddRowMode(column.name, "null")
                        }
                      />
                      NULL
                    </label>
                    <label className="flex items-center gap-1">
                      <input
                        type="radio"
                        name={`add-row-mode-${column.name}`}
                        data-testid={`add-row-mode-default-${column.name}`}
                        disabled={!hasDefault}
                        checked={field.mode === "default"}
                        onChange={() =>
                          handleSetAddRowMode(column.name, "default")
                        }
                      />
                      default
                    </label>
                  </div>
                </div>
              );
            })}
          </div>
          <div className="flex items-center justify-end gap-2">
            <Button
              size="sm"
              className="h-8 px-3 text-xs"
              disabled={isWriting}
              onClick={() => {
                void handleSubmitAddRow();
              }}
            >
              {isWriting ? "Inserting…" : "Insert"}
            </Button>
          </div>
        </div>
      ) : null}
      <div className="flex-1 min-w-0 overflow-hidden">
        {viewMode === "data" ? (
          <DataGrid
            data={rows}
            columns={columns}
            edits={currentEdits}
            onEdit={(rowIndex, colIndex, value) =>
              setTableEdit(tableName, rowIndex, colIndex, value)
            }
            hasEdits={hasEdits}
            readOnly={isReadOnly}
            isSaving={commitStatus?.state === "running"}
            onDiscard={() => discardTableEdits(tableName)}
            onSave={() => {
              void commitTableEdits(tableName);
            }}
            onOpenSQL={() => openQueryForTable(tab.schema, tab.table ?? "")}
            viewMode={viewMode}
            onViewModeChange={setViewMode}
            onToggleSidebar={toggleLeftSidebar}
            exportFilenameBase={exportFilenameBase}
            rowSelection={rowSelection}
            onRowSelectionChange={setRowSelection}
            toolbarLeading={
              <div className="flex items-center gap-2">
                <Button
                  size="sm"
                  className="h-9 gap-2 bg-foreground text-background hover:bg-foreground/90"
                  disabled={!canAddRow}
                  onClick={handleOpenAddRow}
                >
                  <IconPlus className="size-3.5" /> Add row
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-9 gap-2 border-destructive/40 text-destructive hover:bg-destructive/10"
                  disabled={!canDeleteSelected}
                  onClick={() => {
                    void handleDeleteSelected();
                  }}
                >
                  <IconTrash className="size-3.5" /> Delete selected
                </Button>
              </div>
            }
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
              connectionId={tab.connectionId}
              schema={tab.schema}
              tableName={tab.table ?? ""}
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
  const { tablePreviews } = useAppStore();

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
            <SchemaRelationshipMap
              connectionId={tab.connectionId}
              schema={tab.schema}
              activeTable={activeTable}
              isClient={isClient}
            />
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
