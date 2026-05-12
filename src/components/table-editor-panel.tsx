import {
  IconChevronLeft,
  IconChevronRight,
  IconChevronsLeft,
  IconChevronsRight,
  IconMaximize,
  IconPlus,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import type { RowSelectionState } from "@tanstack/react-table";
import { useEffect, useMemo, useState } from "react";

import { DataGrid } from "@/components/data-grid";
import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { AddRowForm } from "@/components/table-editor/add-row-form";
import {
  type SubTab,
  TableEditorHeader,
} from "@/components/table-editor/header";
import { RowDetailsPanel } from "@/components/table-editor/row-details-panel";
import { TableStatusBanners } from "@/components/table-editor/status-banners";
import { useRowDetailsVisibility } from "@/components/table-editor/use-row-details-visibility";
import { useTablePagination } from "@/components/table-editor/use-table-pagination";
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
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import { pickRowIdentity } from "@/lib/row-identity";
import {
  type EditOutcome,
  type TablePreviewData,
  tableDataKey,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { useContainerWidth } from "@/lib/use-resizable-width";
import { cn } from "@/lib/utils";

interface TableEditorPanelProps {
  tab: WorkspaceTab;
}

export function TableEditorPanel({ tab }: TableEditorPanelProps) {
  const [activeSubTab, setActiveSubTab] = useState<SubTab>("data");
  const [bodyRef, bodyWidth] = useContainerWidth<HTMLDivElement>();

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
  const rowDetails = useRowDetailsVisibility(bodyWidth);

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

  const pagination = useTablePagination({
    tab,
    data: activeTableData,
    loadTableData,
  });

  const onRefresh = () => {
    if (dataKey) {
      void refreshTableData(dataKey);
    }
  };

  const handleSubmitAddRow = async (values: InsertRowPayloadEntry[]) => {
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

  const selectedRowIndex =
    selectedRowIndices.length === 1 ? selectedRowIndices[0] : null;
  const selectedRow =
    selectedRowIndex !== null ? rows[selectedRowIndex] : rows[0];

  const rowCountLabel =
    pagination.totalRows !== undefined
      ? `${pagination.totalRows.toLocaleString()} rows`
      : `${rows.length.toLocaleString()} rows`;

  const statusItems: StatusBarItem[] = [
    {
      id: "query",
      label: "Query",
      tone: errorMessage ? "danger" : "healthy",
      value:
        pagination.runtimeMs !== undefined
          ? `Completed · ${pagination.runtimeMs} ms`
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
      value: pagination.totalPages
        ? `${pagination.page} of ${pagination.totalPages}`
        : `${pagination.page}`,
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
      />

      <TableStatusBanners
        errorMessage={errorMessage}
        showReadOnlyBanner={isReadOnly && Boolean(activeTableStructure)}
        commitStatus={commitStatus}
        lastOutcome={lastOutcome}
        onRetryLoad={onRefresh}
        onDismissOutcome={() => setLastOutcome(null)}
      />

      {isAddRowOpen && activeTableStructure ? (
        <AddRowForm
          columns={activeTableStructure.columns}
          isWriting={isWriting}
          onSubmit={handleSubmitAddRow}
          onClose={() => setIsAddRowOpen(false)}
        />
      ) : null}

      {/* Body */}
      <div
        ref={bodyRef}
        data-workspace-density={
          bodyWidth > 0 && bodyWidth < 760 ? "compact" : "cozy"
        }
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
        <div className="min-w-0 flex-1 overflow-hidden">
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
                    onClick={() => setIsAddRowOpen(true)}
                    aria-label="Add row"
                    title="Add row"
                  >
                    <IconPlus className="size-3.5" />{" "}
                    <span className="dbunk-primary-label">Add row</span>
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    className="border-danger/40 text-danger hover:bg-danger/10"
                    disabled={!canDeleteSelected}
                    aria-label="Delete selected"
                    title="Delete selected"
                    onClick={() => {
                      void handleDeleteSelected();
                    }}
                  >
                    <IconTrash className="size-3.5" />{" "}
                    <span className="dbunk-primary-label">Delete selected</span>
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

        {activeSubTab === "data" ? (
          <RowDetailsPanel
            columns={columns}
            selectedRow={selectedRow}
            selectedRowIndex={selectedRowIndex}
            selectedRowCount={selectedRowIndices.length}
            totalRows={pagination.totalRows ?? rows.length}
            indexes={activeTableStructure?.indexes.length ?? 0}
            bodyWidth={bodyWidth}
            wideVisible={rowDetails.isOpen}
            overlayOpen={rowDetails.overlayOpen}
            onOverlayOpenChange={rowDetails.setOverlayOpen}
            onClose={rowDetails.onClose}
          />
        ) : null}
      </div>

      {/* Pagination footer (data only) */}
      {activeSubTab === "data" && tab.kind === "table" ? (
        <div
          data-testid="table-pagination"
          className="flex h-8 shrink-0 items-center justify-between gap-2 border-t border-border-subtle bg-surface-window px-3 text-[0.6875rem] text-text-muted"
        >
          <span className="tabular-nums">
            Showing {pagination.startRow.toLocaleString()} to{" "}
            {pagination.endRow.toLocaleString()} of{" "}
            {(pagination.totalRows ?? rows.length).toLocaleString()} rows
          </span>
          <Pagination
            page={pagination.page}
            totalPages={pagination.totalPages}
            isLastPage={pagination.isLastPage}
            isLoading={isLoading}
            onFirst={pagination.onFirstPage}
            onPrev={pagination.onPrevPage}
            onNext={pagination.onNextPage}
            onLast={pagination.onLastPage}
            onJump={pagination.goToPage}
          />
          <span className="tabular-nums">{pagination.pageSize} rows</span>
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
        className="size-6"
      >
        <IconChevronsLeft className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Previous page"
        onClick={onPrev}
        disabled={page <= 1 || isLoading}
        className="size-6"
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
                "h-6 min-w-6 rounded-sm px-1.5 text-xs tabular-nums transition-colors",
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
        className="size-6"
      >
        <IconChevronRight className="size-3.5" />
      </Button>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label="Last page"
        onClick={onLast}
        disabled={isLastPage || isLoading || totalPages === undefined}
        className="size-6"
      >
        <IconChevronsRight className="size-3.5" />
      </Button>
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
          <div className="flex h-10 shrink-0 items-center justify-between border-b border-border-subtle bg-surface-window px-3">
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
