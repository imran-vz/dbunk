import { IconMaximize, IconX } from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { StatusBar } from "@/components/status-bar";
import { AddRowForm } from "@/components/table-editor/add-row-form";
import { TableEditorBody } from "@/components/table-editor/body";
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
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import {
  type EditOutcome,
  type TablePreviewData,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
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

  const {
    tableEdits,
    openQueryForTable,
    loadTableData,
    refreshTableData,
    setTableEdit,
    discardTableEdits,
    commitTableEdits,
    addTableRow,
    deleteSelectedTableRows,
  } = useAppStore();

  const [isAddRowOpen, setIsAddRowOpen] = useState(false);
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
      />

      <TableStatusBanners
        errorMessage={errorMessage}
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
        onOpenSql={() => openQueryForTable(tab.schema, tab.table ?? "")}
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
      />

      <StatusBar items={statusItems} />
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
