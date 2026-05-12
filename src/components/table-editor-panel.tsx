import {
  IconAlertTriangle,
  IconCheck,
  IconChevronLeft,
  IconChevronRight,
  IconChevronsLeft,
  IconChevronsRight,
  IconDotsVertical,
  IconLayoutSidebarRight,
  IconLock,
  IconMaximize,
  IconPlus,
  IconTable,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import type { RowSelectionState } from "@tanstack/react-table";
import { useEffect, useMemo, useState } from "react";

import { DataGrid } from "@/components/data-grid";
import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { TableStructureView } from "@/components/table-structure-view";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  buildInsertValuesPayload,
  type InsertRowFieldMode,
  type InsertRowFormState,
  initialFormState,
} from "@/lib/insert-row-form";
import { pickRowIdentity } from "@/lib/row-identity";
import {
  type EditOutcome,
  type TablePreviewData,
  tableDataKey,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface TableEditorPanelProps {
  tab: WorkspaceTab;
}

type SubTab = "data" | "schema" | "indexes" | "relations";

const SUB_TABS: Array<{ id: SubTab; label: string }> = [
  { id: "data", label: "Data" },
  { id: "schema", label: "Schema" },
  { id: "indexes", label: "Indexes" },
  { id: "relations", label: "Relations" },
];

export function TableEditorPanel({ tab }: TableEditorPanelProps) {
  const [activeSubTab, setActiveSubTab] = useState<SubTab>("data");
  const [isRowDetailsOpen, setIsRowDetailsOpen] = useState(true);

  const tableName = tab.kind === "table" ? (tab.table ?? "") : "";
  // Sliced subscription: re-render only when *this* table's lifecycle
  // slot changes, not when any other table's status moves.
  const commitStatus = useAppStore((s) => s.tableEditsCommitStatus[tableName]);
  // Terminal outcome lives component-local. Disappears on tab unmount,
  // which is the intended trade-off (CONTEXT.md — Edit Outcome).
  const [lastOutcome, setLastOutcome] = useState<EditOutcome | null>(null);

  const {
    tableData,
    tableStructure,
    tableLoadStatus,
    tableEdits,
    openQueryForTable,
    loadTableData,
    loadTableStructure,
    refreshTableData,
    setTableEdit,
    discardTableEdits,
    commitTableEdits,
    addTableRow,
    deleteSelectedTableRows,
  } = useAppStore();

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
    // Reset the terminal-outcome badge on table switch — the panel
    // instance is reused across tab switches (no React key), so without
    // this the badge from table A could leak into table B's view.
    setLastOutcome(null);
    if (tab.kind === "table" && tab.table && tab.connectionId) {
      void loadTableData(tab.connectionId, tab.schema, tab.table);
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
  const status = tableName ? tableLoadStatus[tableName] : undefined;
  const currentEdits = tableEdits[tableName];
  const hasEdits = Object.keys(currentEdits ?? {}).length > 0;
  const rowIdentity = pickRowIdentity(activeTableStructure);
  const isReadOnly = rowIdentity === null;
  const connections = useAppStore((s) => s.connections);
  const connection = connections.find((c) => c.id === tab.connectionId);
  const structureLoaded = Boolean(activeTableStructure);
  const isWriting =
    commitStatus?.state === "running" || commitStatus?.state === "queued";
  // Mutation gates come from the per-table capability flags rather than
  // an engine-name literal — that way a CH MergeTree table is editable
  // and a CH Distributed/View table is not, with the same shape of code.
  const capabilities = activeTableStructure?.capabilities;
  const canInsertRows = capabilities?.canInsertRows ?? false;
  const canUpdateRows = capabilities?.canUpdateRows ?? false;
  const canDeleteRows = capabilities?.canDeleteRows ?? false;
  const selectedRowIndices = useMemo(
    () =>
      Object.entries(rowSelection)
        .filter(([, selected]) => selected)
        .map(([rowId]) => Number.parseInt(rowId, 10))
        .filter((n) => Number.isFinite(n)),
    [rowSelection],
  );
  const canDeleteSelected =
    selectedRowIndices.length > 0 && canDeleteRows && !isReadOnly && !isWriting;
  const canAddRow = structureLoaded && canInsertRows && !isWriting;
  // Used to gate the inline cell editor; surface it on the same axis as
  // the other capability-derived flags.
  const canEditCells = canUpdateRows && !isReadOnly && !isWriting;

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

  const goToPage = (next: number) => {
    if (tab.kind !== "table" || !tab.table || !tab.connectionId) return;
    const target = Math.max(1, totalPages ? Math.min(totalPages, next) : next);
    if (target === page) return;
    void loadTableData(
      tab.connectionId,
      tab.schema,
      tab.table,
      target,
      pageSize,
    );
  };

  const onPrevPage = () => goToPage(page - 1);
  const onNextPage = () => goToPage(page + 1);
  const onFirstPage = () => goToPage(1);
  const onLastPage = () => {
    if (totalPages !== undefined) {
      goToPage(totalPages);
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
    const outcome = await addTableRow(tableName, values);
    setLastOutcome(outcome);
    if (outcome.kind === "completed") {
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
    const outcome = await deleteSelectedTableRows(
      tableName,
      selectedRowIndices,
    );
    setLastOutcome(outcome);
    if (outcome.kind === "completed") {
      setRowSelection({});
    }
  };

  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;

  const hasMultipleSelectedRows = selectedRowIndices.length > 1;
  const selectedRowIndex =
    selectedRowIndices.length === 1 ? selectedRowIndices[0] : null;
  const selectedRow =
    selectedRowIndex !== null ? rows[selectedRowIndex] : rows[0];

  const rowCountLabel =
    totalRows !== undefined
      ? `${totalRows.toLocaleString()} rows`
      : `${rows.length.toLocaleString()} rows`;

  const startRow =
    totalRows === undefined && rows.length === 0
      ? 0
      : (page - 1) * pageSize + 1;
  const endRow =
    totalRows !== undefined
      ? Math.min(totalRows, page * pageSize)
      : (page - 1) * pageSize + rows.length;

  const statusItems: StatusBarItem[] = [
    {
      id: "query",
      label: "Query",
      tone: errorMessage ? "danger" : "healthy",
      value:
        runtimeMs !== undefined
          ? `Completed · ${runtimeMs} ms`
          : isLoading
            ? "Loading…"
            : "Idle",
    },
    {
      id: "data",
      label: "Data",
      value: `${rows.length.toLocaleString()} rows`,
    },
    {
      id: "page",
      label: "Page",
      value: totalPages ? `${page} of ${totalPages}` : `${page}`,
    },
    {
      id: "connection",
      label: "Connection",
      tone: "healthy",
      value: connection?.status ?? "Healthy",
      align: "right",
    },
  ];

  return (
    <div className="flex h-full flex-col bg-surface-app">
      {isLoading ? (
        <div
          data-testid="table-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}

      {/* Header */}
      <div className="shrink-0 border-b border-border-subtle bg-surface-window px-5 pt-4">
        <div className="flex items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex size-8 items-center justify-center rounded-md border border-accent-green/30 bg-accent-green/10 text-accent-green">
              <IconTable className="size-4" />
            </div>
            <h1 className="truncate text-lg font-semibold tracking-tight text-foreground">
              {tab.table ?? tab.label}
            </h1>
            <Badge variant="outline" className="h-6 rounded-md px-2">
              {rowCountLabel}
            </Badge>
            <Badge variant="outline" className="h-6 rounded-md px-2">
              {tab.schema}
            </Badge>
          </div>
          <div className="flex items-center gap-2">
            {activeSubTab === "data" ? (
              <Button
                size="sm"
                variant={isRowDetailsOpen ? "secondary" : "outline"}
                aria-pressed={isRowDetailsOpen}
                aria-label={
                  isRowDetailsOpen ? "Hide row details" : "Show row details"
                }
                onClick={() => setIsRowDetailsOpen((open) => !open)}
              >
                <IconLayoutSidebarRight className="size-3.5" />
                Details
              </Button>
            ) : null}
            <DropdownMenu>
              <DropdownMenuTrigger
                aria-label="Table actions"
                className={cn(
                  "inline-flex h-8 items-center gap-2 rounded-md border border-border-subtle bg-surface-panel px-3 text-xs font-medium text-foreground transition-colors hover:bg-surface-panel-elevated",
                )}
              >
                <IconDotsVertical className="size-3.5 text-text-muted" />
                Table actions
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                <DropdownMenuItem
                  onClick={() => openQueryForTable(tab.schema, tab.table ?? "")}
                >
                  Open in SQL
                </DropdownMenuItem>
                <DropdownMenuItem onClick={onRefresh}>
                  Refresh data
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
        <div className="mt-3 flex items-end gap-1">
          {SUB_TABS.map(({ id, label }) => {
            const isActive = activeSubTab === id;
            return (
              <button
                key={id}
                type="button"
                aria-current={isActive ? "page" : undefined}
                onClick={() => {
                  setActiveSubTab(id);
                }}
                className={cn(
                  "relative h-9 px-3 text-sm font-medium transition-colors",
                  isActive
                    ? "text-foreground"
                    : "text-text-muted hover:text-foreground",
                )}
              >
                {label}
                {isActive ? (
                  <span className="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-accent-green" />
                ) : null}
              </button>
            );
          })}
        </div>
      </div>

      {errorMessage ? (
        <div
          data-testid="table-error"
          role="alert"
          className="flex items-center gap-2 border-b border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
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
          className="flex items-center gap-2 border-b border-warning/40 bg-warning/10 px-4 py-2 text-xs text-warning"
        >
          <IconLock className="size-4" />
          <span>
            This table has no primary key or non-null unique index — it is
            read-only. Add a unique constraint to enable editing.
          </span>
        </output>
      ) : null}

      {commitStatus?.state === "queued" ? (
        <output
          data-testid="table-commit-queued"
          className="flex items-center gap-2 border-b border-warning/40 bg-warning/10 px-4 py-2 text-xs text-warning"
        >
          <IconLock className="size-4" />
          <span>
            Queued — applying {commitStatus.mutationIds.length} mutation
            {commitStatus.mutationIds.length === 1 ? "" : "s"} in the
            background. Refreshing when complete.
          </span>
        </output>
      ) : null}

      {lastOutcome?.kind === "completed" &&
      lastOutcome.rowsAffected !== undefined ? (
        <output
          data-testid="table-commit-success"
          className="flex items-center gap-2 border-b border-accent-green/40 bg-accent-green/10 px-4 py-2 text-xs text-accent-green-hover"
        >
          <IconCheck className="size-4" />
          <span>
            Saved {lastOutcome.rowsAffected} row
            {lastOutcome.rowsAffected === 1 ? "" : "s"} in{" "}
            {lastOutcome.runtimeMs} ms.
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() => setLastOutcome(null)}
          >
            Dismiss
          </Button>
        </output>
      ) : null}

      {lastOutcome?.kind === "failed" ? (
        <div
          data-testid="table-commit-error"
          role="alert"
          className="flex items-center gap-2 border-b border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
        >
          <IconX className="size-4" />
          <span>Failed to save: {lastOutcome.reason}</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() => setLastOutcome(null)}
          >
            Dismiss
          </Button>
        </div>
      ) : null}

      {lastOutcome?.kind === "timeout" ? (
        <div
          data-testid="table-commit-timeout"
          role="alert"
          className="flex items-center gap-2 border-b border-warning/40 bg-warning/10 px-4 py-2 text-xs text-warning"
        >
          <IconAlertTriangle className="size-4" />
          <span>
            Mutation did not complete in time. Check system.mutations for{" "}
            {lastOutcome.remaining.length} remaining.
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={() => setLastOutcome(null)}
          >
            Dismiss
          </Button>
        </div>
      ) : null}

      {isAddRowOpen && activeTableStructure ? (
        <div
          data-testid="add-row-form"
          className="flex flex-col gap-2 border-b border-border-subtle bg-surface-panel px-4 py-3"
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
                  className="flex flex-col gap-1 rounded-md border border-border-subtle bg-surface-app px-2 py-1.5"
                >
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-medium">{column.name}</span>
                    <span className="text-[0.625rem] text-text-muted">
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
                  <div className="flex items-center gap-3 text-[0.625rem] text-text-muted">
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

      {/* Body */}
      <div
        className={cn(
          "grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)] overflow-hidden",
          isRowDetailsOpen &&
            activeSubTab === "data" &&
            "xl:grid-cols-[minmax(0,1fr)_20rem]",
        )}
      >
        <div className="min-w-0 overflow-hidden">
          {activeSubTab === "data" ? (
            <DataGrid
              data={rows}
              columns={columns}
              edits={currentEdits}
              onEdit={(rowIndex, colIndex, value) =>
                setTableEdit(tableName, rowIndex, colIndex, value)
              }
              hasEdits={hasEdits}
              readOnly={isReadOnly || !canEditCells}
              isSaving={commitStatus?.state === "running"}
              onDiscard={() => discardTableEdits(tableName)}
              onSave={async () => {
                const outcome = await commitTableEdits(tableName);
                setLastOutcome(outcome);
              }}
              onOpenSQL={() => openQueryForTable(tab.schema, tab.table ?? "")}
              onRefresh={onRefresh}
              exportFilenameBase={exportFilenameBase}
              rowSelection={rowSelection}
              onRowSelectionChange={setRowSelection}
              toolbarLeading={
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    disabled={!canAddRow}
                    onClick={handleOpenAddRow}
                  >
                    <IconPlus className="size-3.5" /> Add row
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="border-danger/40 text-danger hover:bg-danger/10"
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
          ) : activeSubTab === "schema" ? (
            <TableStructureView
              connectionId={tab.connectionId}
              schema={tab.schema}
              tableName={tab.table ?? ""}
              className="h-full"
            />
          ) : (
            <SubTabPlaceholder kind={activeSubTab} />
          )}
        </div>

        {isRowDetailsOpen && activeSubTab === "data" ? (
          <aside
            data-testid="row-details-panel"
            className="hidden min-h-0 flex-col gap-3 border-l border-border-subtle bg-surface-window p-4 text-xs xl:flex"
          >
            <div className="flex items-start justify-between">
              <div>
                <div className="flex items-center gap-2">
                  <span className="text-sm font-semibold text-foreground">
                    Row {selectedRowIndex !== null ? selectedRowIndex + 1 : 1}
                  </span>
                  <Badge variant="success" className="h-5 px-2">
                    Selected
                  </Badge>
                </div>
                <div className="mt-0.5 text-text-muted">
                  {hasMultipleSelectedRows
                    ? `${selectedRowIndices.length} rows selected`
                    : selectedRowIndices.length === 1
                      ? "1 selected"
                      : "First visible row"}
                </div>
              </div>
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label="Close row details"
                onClick={() => setIsRowDetailsOpen(false)}
              >
                <IconX className="size-3.5" />
              </Button>
            </div>

            <div className="flex flex-1 flex-col gap-2 overflow-auto">
              {hasMultipleSelectedRows ? (
                <div className="rounded-md border border-accent-green/20 bg-accent-green/10 p-3 text-text-muted">
                  <div className="font-semibold text-foreground">
                    Multiple rows selected
                  </div>
                  <div className="mt-1">
                    Select a single row to inspect column values.
                  </div>
                </div>
              ) : selectedRow ? (
                columns.map((column, index) => (
                  <div
                    key={column}
                    className="rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2"
                  >
                    <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
                      {column}
                    </div>
                    <div className="mt-1 truncate font-mono text-[0.75rem] text-foreground">
                      {selectedRow[index] || "NULL"}
                    </div>
                  </div>
                ))
              ) : (
                <div className="rounded-md border border-border-subtle bg-surface-panel-elevated p-3 text-text-muted">
                  No row selected
                </div>
              )}
            </div>

            <SummaryCard
              totalRows={totalRows ?? rows.length}
              indexes={activeTableStructure?.indexes.length ?? 0}
            />
          </aside>
        ) : null}
      </div>

      {/* Pagination footer (data only) */}
      {activeSubTab === "data" && tab.kind === "table" ? (
        <div
          data-testid="table-pagination"
          className="flex h-11 shrink-0 items-center justify-between gap-3 border-t border-border-subtle bg-surface-window px-4 text-xs text-text-muted"
        >
          <span className="tabular-nums">
            Showing {startRow.toLocaleString()} to {endRow.toLocaleString()} of{" "}
            {(totalRows ?? rows.length).toLocaleString()} rows
          </span>
          <Pagination
            page={page}
            totalPages={totalPages}
            isLastPage={isLastPage}
            isLoading={isLoading}
            onFirst={onFirstPage}
            onPrev={onPrevPage}
            onNext={onNextPage}
            onLast={onLastPage}
            onJump={goToPage}
          />
          <span className="tabular-nums">{pageSize} rows</span>
        </div>
      ) : null}

      <StatusBar items={statusItems} />
    </div>
  );
}

function SubTabPlaceholder({ kind }: { kind: "indexes" | "relations" }) {
  const titles = {
    indexes: "Indexes",
    relations: "Relations",
  } as const;
  const descriptions = {
    indexes:
      "Per-table indexes (name, columns, type, unique flag, size) will appear here.",
    relations:
      "Foreign-key relationships in and out of this table will be shown here.",
  } as const;
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="max-w-md rounded-lg border border-dashed border-border-subtle bg-surface-panel/50 p-6 text-center">
        <div className="text-sm font-semibold text-foreground">
          {titles[kind]}
        </div>
        <p className="mt-1 text-xs text-text-muted">{descriptions[kind]}</p>
        <p className="mt-3 text-[0.625rem] uppercase tracking-[0.12em] text-text-muted">
          Coming soon
        </p>
      </div>
    </div>
  );
}

function Pagination({
  page,
  totalPages,
  isLastPage,
  isLoading,
  onFirst,
  onPrev,
  onNext,
  onLast,
  onJump,
}: {
  page: number;
  totalPages: number | undefined;
  isLastPage: boolean;
  isLoading: boolean;
  onFirst: () => void;
  onPrev: () => void;
  onNext: () => void;
  onLast: () => void;
  onJump: (page: number) => void;
}) {
  const pageButtons = useMemo(() => {
    if (!totalPages) return [page];
    const pages: Array<number | "ellipsis-left" | "ellipsis-right"> = [];
    const window = 1;
    pages.push(1);
    if (page - window > 2) pages.push("ellipsis-left");
    for (
      let p = Math.max(2, page - window);
      p <= Math.min(totalPages - 1, page + window);
      p++
    ) {
      pages.push(p);
    }
    if (page + window < totalPages - 1) pages.push("ellipsis-right");
    if (totalPages > 1) pages.push(totalPages);
    return pages;
  }, [page, totalPages]);

  return (
    <div className="flex items-center gap-1">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="First page"
        onClick={onFirst}
        disabled={page <= 1 || isLoading}
        className="size-8"
      >
        <IconChevronsLeft className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Previous page"
        onClick={onPrev}
        disabled={page <= 1 || isLoading}
        className="size-8"
      >
        <IconChevronLeft className="size-3.5" />
      </Button>
      <div className="flex items-center gap-0.5 px-1">
        {pageButtons.map((entry, idx) =>
          typeof entry === "number" ? (
            <button
              type="button"
              key={`p-${entry}`}
              aria-label={`Go to page ${entry}`}
              aria-current={entry === page ? "page" : undefined}
              onClick={() => onJump(entry)}
              className={cn(
                "h-7 min-w-7 rounded-md px-2 text-xs tabular-nums transition-colors",
                entry === page
                  ? "bg-accent-green/15 text-accent-green-hover"
                  : "text-text-muted hover:bg-surface-panel-elevated hover:text-foreground",
              )}
            >
              {entry}
            </button>
          ) : (
            <span
              // biome-ignore lint/suspicious/noArrayIndexKey: ellipsis position is stable
              key={`${entry}-${idx}`}
              className="px-1 text-text-muted"
            >
              …
            </span>
          ),
        )}
      </div>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Next page"
        onClick={onNext}
        disabled={isLastPage || isLoading}
        className="size-8"
      >
        <IconChevronRight className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Last page"
        onClick={onLast}
        disabled={isLastPage || isLoading || totalPages === undefined}
        className="size-8"
      >
        <IconChevronsRight className="size-3.5" />
      </Button>
    </div>
  );
}

function SummaryCard({
  totalRows,
  indexes,
}: {
  totalRows: number;
  indexes: number;
}) {
  const rows: Array<[string, string]> = [
    ["Total rows", totalRows.toLocaleString()],
    ["Data size", "—"],
    ["Indexes", indexes.toLocaleString()],
    ["Last vacuum", "—"],
    ["Last analyze", "—"],
  ];
  return (
    <div className="rounded-md border border-border-subtle bg-surface-panel-elevated p-3">
      <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
        Summary
      </div>
      <dl className="mt-2 space-y-1.5 text-xs">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center justify-between">
            <dt className="text-text-muted">{label}</dt>
            <dd className="tabular-nums text-foreground">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

export interface TableSidebarProps {
  tab: WorkspaceTab;
  isClient: boolean;
}

export function TableSidebar({ tab, isClient }: TableSidebarProps) {
  const { tablePreviews } = useAppStore();
  const [isSchemaMapFullscreen, setIsSchemaMapFullscreen] = useState(false);

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
          <div className="flex h-12 shrink-0 items-center justify-between border-b border-border-subtle bg-surface-window px-4">
            <div className="min-w-0">
              <div className="text-sm font-semibold">Schema map</div>
              <div className="truncate text-xs text-text-muted">
                {tab.schema}
                {activeTable ? ` / ${activeTable}` : ""}
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
          <div className="min-h-0 flex-1 p-3">
            <div className="h-full overflow-hidden rounded-md border border-border-subtle bg-surface-window">
              <SchemaRelationshipMap
                connectionId={tab.connectionId}
                schema={tab.schema}
                activeTable={activeTable}
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
