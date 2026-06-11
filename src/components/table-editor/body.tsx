import { IconPlus, IconTrash } from "@tabler/icons-react";
import type * as React from "react";
import { useMemo } from "react";

import {
  type ColumnHeaderMeta,
  DataGrid,
  type ForeignKeyTarget,
} from "@/components/data-grid";
import { TableStructureView } from "@/components/table-structure-view";
import { Button } from "@/components/ui/button";
import type {
  ExportCompression,
  ExportEncoding,
  ExportFormat,
} from "@/lib/export";
import type { TableDataState, TableStructure } from "@/lib/store";
import type { SelectedTableSessionCapabilities } from "@/lib/table-session";

import type { SubTab } from "./header";
import { IndexesSubTab } from "./indexes-sub-tab";
import { Pagination } from "./pagination";
import { RelationsSubTab } from "./relations-sub-tab";
import { RowDetailsPanel } from "./row-details-panel";
import { SchemaMapSubTab } from "./schema-map-sub-tab";
import { SpecializedEditors } from "./specialized-editors";
import type { TableRef } from "./table-ref";
import type { RowDetailsVisibility } from "./use-row-details-visibility";
import type { RowSelection } from "./use-row-selection";
import type { TablePagination } from "./use-table-pagination";

interface TableEditorBodyProps {
  bodyRef: React.Ref<HTMLDivElement>;
  bodyWidth: number;
  activeSubTab: SubTab;
  tableRef: TableRef | null;
  schema: string;
  connectionId: string;
  tableName: string;
  data: TableDataState | undefined;
  structure: TableStructure | undefined;
  currentEdits: Record<number, Record<number, string>> | undefined;
  hasEdits: boolean;
  selection: RowSelection<string[]>;
  caps: SelectedTableSessionCapabilities;
  rowDetails: RowDetailsVisibility;
  pagination: TablePagination;
  isLoading: boolean;
  isSaving: boolean;
  exportFilenameBase: string;
  onRefresh: () => void;
  onOpenAddRow: () => void;
  onOpenImport: () => void;
  onOpenSql: () => void;
  onOpenTable: (schema: string, tableName: string) => void;
  onSubTabChange: (next: SubTab) => void;
  onCellEdit: (rowIndex: number, colIndex: number, value: string) => void;
  onDiscardEdits: () => void;
  onSaveEdits: () => Promise<void>;
  onDeleteSelected: () => void;
  onFollowForeignKey?: (
    rowIndex: number,
    target: ForeignKeyTarget,
    value: string,
  ) => void;
  rowExpansion?: { rowIndex: number; content: React.ReactNode } | null;
  onExportWholeTable?: (options: {
    format: ExportFormat;
    encoding: ExportEncoding;
    compression: ExportCompression;
    nullAs: string;
  }) => Promise<void>;
  onSaveExportTask?: (options: {
    format: ExportFormat;
    encoding: ExportEncoding;
    compression: ExportCompression;
    nullAs: string;
  }) => void;
  onRunSavedExportTask?: () => Promise<void>;
  hasSavedExportTask?: boolean;
}

export function TableEditorBody({
  bodyRef,
  bodyWidth,
  activeSubTab,
  tableRef,
  schema,
  connectionId,
  tableName,
  data,
  structure,
  currentEdits,
  hasEdits,
  selection,
  caps,
  rowDetails,
  pagination,
  isLoading,
  isSaving,
  exportFilenameBase,
  onRefresh,
  onOpenAddRow,
  onOpenImport,
  onOpenSql,
  onOpenTable,
  onSubTabChange,
  onCellEdit,
  onDiscardEdits,
  onSaveEdits,
  onDeleteSelected,
  onFollowForeignKey,
  rowExpansion,
  onExportWholeTable,
  onSaveExportTask,
  onRunSavedExportTask,
  hasSavedExportTask,
}: TableEditorBodyProps) {
  const columns = data?.columns ?? [];
  const rows = data?.rows ?? [];
  const columnTypes = columns.map(
    (name) => structure?.columns.find((c) => c.name === name)?.dataType,
  );
  const columnMetadata = useMemo<Array<ColumnHeaderMeta | undefined>>(
    () => buildColumnMetadata(columns, structure),
    [columns, structure],
  );
  const density = bodyWidth > 0 && bodyWidth < 760 ? "compact" : "cozy";
  const showFooter = activeSubTab === "data" && tableRef !== null;

  return (
    <>
      <div
        ref={bodyRef}
        data-workspace-density={density}
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
        <div className="min-w-0 flex-1 overflow-hidden">
          {activeSubTab === "data" ? (
            <DataGrid
              data={rows}
              columns={columns}
              columnTypes={columnTypes}
              columnMetadata={columnMetadata}
              onFollowForeignKey={onFollowForeignKey}
              rowExpansion={rowExpansion}
              edits={currentEdits}
              onEdit={onCellEdit}
              hasEdits={hasEdits}
              readOnly={caps.isReadOnly || !caps.canEditCells}
              isSaving={isSaving}
              onDiscard={onDiscardEdits}
              onSave={onSaveEdits}
              onOpenSQL={onOpenSql}
              onRefresh={onRefresh}
              exportFilenameBase={exportFilenameBase}
              rowSelection={selection.rowSelection}
              onRowSelectionChange={selection.setRowSelection}
              onExportWholeTable={onExportWholeTable}
              onSaveExportTask={onSaveExportTask}
              onRunSavedExportTask={onRunSavedExportTask}
              hasSavedExportTask={hasSavedExportTask}
              toolbarLeading={
                <DataToolbar
                  canAddRow={caps.canAddRow}
                  canDeleteSelected={caps.canDeleteSelected}
                  onOpenAddRow={onOpenAddRow}
                  onOpenImport={onOpenImport}
                  onDeleteSelected={onDeleteSelected}
                />
              }
            />
          ) : activeSubTab === "schema" ? (
            <TableStructureView
              connectionId={connectionId}
              schema={schema}
              tableName={tableName}
              className="h-full"
            />
          ) : activeSubTab === "specialized" ? (
            <SpecializedEditors
              schema={schema}
              table={tableName}
              connectionId={connectionId}
              structure={structure}
            />
          ) : activeSubTab === "indexes" ? (
            <IndexesSubTab
              connectionId={connectionId}
              schema={schema}
              tableName={tableName}
              onOpenSpecialized={onSubTabChange}
            />
          ) : activeSubTab === "schema-map" ? (
            <SchemaMapSubTab
              connectionId={connectionId}
              schema={schema}
              tableName={tableName}
            />
          ) : (
            <RelationsSubTab
              connectionId={connectionId}
              schema={schema}
              tableName={tableName}
              onOpenTable={onOpenTable}
              onOpenSpecialized={onSubTabChange}
            />
          )}
        </div>

        {activeSubTab === "data" ? (
          <RowDetailsPanel
            columns={columns}
            selectedRow={selection.selectedRow}
            selectedRowIndex={selection.selectedIndex}
            selectedRowCount={selection.selectedCount}
            totalRows={pagination.totalRows ?? rows.length}
            indexes={structure?.indexes.length ?? 0}
            bodyWidth={bodyWidth}
            wideVisible={rowDetails.isOpen}
            overlayOpen={rowDetails.overlayOpen}
            onOverlayOpenChange={rowDetails.setOverlayOpen}
            onClose={rowDetails.onClose}
          />
        ) : null}
      </div>

      {showFooter ? (
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
    </>
  );
}

interface DataToolbarProps {
  canAddRow: boolean;
  canDeleteSelected: boolean;
  onOpenAddRow: () => void;
  onOpenImport: () => void;
  onDeleteSelected: () => void;
}

function DataToolbar({
  canAddRow,
  canDeleteSelected,
  onOpenAddRow,
  onOpenImport,
  onDeleteSelected,
}: DataToolbarProps) {
  return (
    <div className="flex items-center gap-2">
      <Button
        size="sm"
        disabled={!canAddRow}
        onClick={onOpenAddRow}
        aria-label="Add row"
        title="Add row"
      >
        <IconPlus className="size-3.5" />{" "}
        <span className="dbunk-primary-label">Add row</span>
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={!canAddRow}
        onClick={onOpenImport}
        aria-label="Import data"
        title="Import data"
      >
        <IconPlus className="size-3.5" />{" "}
        <span className="dbunk-primary-label">Import</span>
      </Button>
      <Button
        size="sm"
        variant="outline"
        className="border-danger/40 text-danger hover:bg-danger/10"
        disabled={!canDeleteSelected}
        aria-label="Delete selected"
        title="Delete selected"
        onClick={onDeleteSelected}
      >
        <IconTrash className="size-3.5" />{" "}
        <span className="dbunk-primary-label">Delete selected</span>
      </Button>
    </div>
  );
}

/**
 * Compute per-column header metadata from the table structure. Each
 * entry aligns with `columnNames` (the data-grid's column list);
 * missing structure leaves the entry undefined so the grid falls
 * back to the bare column name.
 *
 * The cross-engine flags (PK / FK / indexed / unique / not-null /
 * default) are derived purely from `TableStructure` and work for
 * both Postgres and ClickHouse. The `derivationKind` field is
 * ClickHouse-only — Postgres leaves it null.
 */
function buildColumnMetadata(
  columnNames: string[],
  structure: TableStructure | undefined,
): Array<ColumnHeaderMeta | undefined> {
  if (!structure) return columnNames.map(() => undefined);
  const fkColumns = new Set<string>();
  const fkTargetByColumn = new Map<string, string>();
  const fkStructuredTargetByColumn = new Map<
    string,
    { schema: string; table: string; column: string }
  >();
  for (const fk of structure.foreignKeys) {
    for (let i = 0; i < fk.columns.length; i++) {
      const col = fk.columns[i];
      fkColumns.add(col);
      const targetCol = fk.referencedColumns[i] ?? "?";
      fkTargetByColumn.set(
        col,
        `${fk.referencedSchema}.${fk.referencedTable}.${targetCol}`,
      );
      fkStructuredTargetByColumn.set(col, {
        schema: fk.referencedSchema,
        table: fk.referencedTable,
        column: targetCol,
      });
    }
  }
  const indexedColumns = new Set<string>();
  const uniqueColumns = new Set<string>();
  for (const idx of structure.indexes) {
    for (const col of idx.columns) {
      indexedColumns.add(col);
      if (idx.isUnique) uniqueColumns.add(col);
    }
  }
  return columnNames.map((name) => {
    const info = structure.columns.find((c) => c.name === name);
    if (!info) return undefined;
    return {
      isPrimaryKey: info.isPrimaryKey,
      isForeignKey: fkColumns.has(name),
      isIndexed: indexedColumns.has(name) && !info.isPrimaryKey,
      isUnique: uniqueColumns.has(name) && !info.isPrimaryKey,
      notNull: !info.nullable,
      hasDefault: info.defaultValue !== null,
      dataType: info.dataType,
      derivationKind: info.derivationKind ?? null,
      description: fkTargetByColumn.get(name),
      foreignKeyTarget: fkStructuredTargetByColumn.get(name),
    };
  });
}
