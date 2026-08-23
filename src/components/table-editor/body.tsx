import { IconCopy, IconPencil, IconPlus, IconTrash } from "@tabler/icons-react";
import type * as React from "react";
import { useMemo } from "react";

import {
  type ColumnHeaderMeta,
  DataGrid,
  type ForeignKeyTarget,
} from "@/components/data-grid";
import type { ServerBrowseGridModel } from "@/components/data-grid/browse-model";
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
  onEditIntent?: (rowIndex: number, colIndex: number) => void;
  getCellReadOnlyReason?: (
    rowIndex: number,
    colIndex: number,
  ) => string | undefined;
  getRowState?: (
    rowIndex: number,
  ) => "deleted" | "inserted" | "duplicate" | "excluded" | undefined;
  onDiscardEdits: () => void;
  onSaveEdits: () => Promise<void>;
  onDeleteSelected: () => void;
  onDuplicateSelected?: () => void;
  onBulkEditSelected?: () => void;
  stagedChangeCount?: number;
  onOpenReview?: () => void;
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
  serverBrowse?: ServerBrowseGridModel;
  onExpandGrid?: () => void;
  expanded?: boolean;
  reviewPanel?: React.ReactNode;
}

export function TableEditorBody({
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
  onEditIntent,
  getCellReadOnlyReason,
  getRowState,
  onDiscardEdits,
  onSaveEdits,
  onDeleteSelected,
  onDuplicateSelected,
  onBulkEditSelected,
  stagedChangeCount = 0,
  onOpenReview,
  onFollowForeignKey,
  rowExpansion,
  onExportWholeTable,
  onSaveExportTask,
  onRunSavedExportTask,
  hasSavedExportTask,
  serverBrowse,
  onExpandGrid,
  expanded,
  reviewPanel,
}: TableEditorBodyProps) {
  const columns = useMemo(() => data?.columns ?? [], [data?.columns]);
  const rows = data?.rows ?? [];
  const columnTypes = columns.map(
    (name) => structure?.columns.find((c) => c.name === name)?.dataType,
  );
  const columnMetadata = useMemo<Array<ColumnHeaderMeta | undefined>>(
    () => buildColumnMetadata(columns, structure),
    [columns, structure],
  );
  const showFooter = activeSubTab === "data" && tableRef !== null;

  return (
    <>
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
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
              onEditIntent={onEditIntent}
              getCellReadOnlyReason={getCellReadOnlyReason}
              getRowState={getRowState}
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
              serverBrowse={serverBrowse}
              onExpandGrid={onExpandGrid}
              expanded={expanded}
              toolbarLeading={
                <DataToolbar
                  canAddRow={caps.canAddRow}
                  canDeleteSelected={caps.canDeleteSelected}
                  canDuplicateSelected={
                    Boolean(onDuplicateSelected) &&
                    selection.selectedCount === 1
                  }
                  canBulkEditSelected={
                    Boolean(onBulkEditSelected) && selection.selectedCount > 0
                  }
                  onOpenAddRow={onOpenAddRow}
                  onOpenImport={onOpenImport}
                  onDeleteSelected={onDeleteSelected}
                  onDuplicateSelected={onDuplicateSelected}
                  onBulkEditSelected={onBulkEditSelected}
                  stagedChangeCount={stagedChangeCount}
                  onOpenReview={onOpenReview}
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

        {reviewPanel}

        {activeSubTab === "data" && !expanded && !reviewPanel ? (
          <RowDetailsPanel
            columns={columns}
            selectedRow={selection.selectedRow}
            selectedRowIndex={selection.selectedIndex}
            selectedRowCount={selection.selectedCount}
            totalRows={pagination.totalRows ?? rows.length}
            indexes={structure?.indexes.length ?? 0}
            visible={rowDetails.visible}
            onClose={rowDetails.onClose}
          />
        ) : null}
      </div>

      {showFooter && !expanded ? (
        <div
          data-testid="table-pagination"
          className="flex h-8 shrink-0 items-center justify-between gap-2 border-t border-border-subtle bg-surface-window px-3 text-2xs text-text-muted"
        >
          <span className="tabular-nums">
            {serverBrowse
              ? pagination.countLabel
              : `Showing ${pagination.startRow.toLocaleString()} to ${pagination.endRow.toLocaleString()} of ${(pagination.totalRows ?? rows.length).toLocaleString()} rows`}
          </span>
          {serverBrowse && pagination.onCountRows ? (
            <div className="flex items-center gap-1">
              <Button
                size="sm"
                variant="ghost"
                className="h-6 px-2 text-2xs"
                onClick={pagination.onCountRows}
                disabled={pagination.counting || isLoading}
              >
                {pagination.counting ? "Counting…" : "Count rows"}
              </Button>
              {pagination.counting ||
              serverBrowse.countStatus.state === "loading" ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-2xs"
                  onClick={serverBrowse.onCancel}
                >
                  Cancel
                </Button>
              ) : null}
            </div>
          ) : null}
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
            canJump={pagination.canJump}
            countApproximate={pagination.countApproximate}
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
  canDuplicateSelected: boolean;
  canBulkEditSelected: boolean;
  onOpenAddRow: () => void;
  onOpenImport: () => void;
  onDeleteSelected: () => void;
  onDuplicateSelected?: () => void;
  onBulkEditSelected?: () => void;
  stagedChangeCount: number;
  onOpenReview?: () => void;
}

function DataToolbar({
  canAddRow,
  canDeleteSelected,
  canDuplicateSelected,
  canBulkEditSelected,
  onOpenAddRow,
  onOpenImport,
  onDeleteSelected,
  onDuplicateSelected,
  onBulkEditSelected,
  stagedChangeCount,
  onOpenReview,
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
        <IconPlus /> <span className="dbunk-primary-label">Add row</span>
      </Button>
      {onDuplicateSelected ? (
        <Button
          size="sm"
          variant="outline"
          disabled={!canDuplicateSelected}
          onClick={onDuplicateSelected}
          aria-label="Duplicate selected row"
          title="Duplicate selected row"
        >
          <IconCopy />
          <span className="dbunk-primary-label">Duplicate</span>
        </Button>
      ) : null}
      {onBulkEditSelected ? (
        <Button
          size="sm"
          variant="outline"
          disabled={!canBulkEditSelected}
          onClick={onBulkEditSelected}
          aria-label="Bulk edit selected rows"
          title="Bulk edit selected rows"
        >
          <IconPencil />
          <span className="dbunk-primary-label">Bulk edit</span>
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="outline"
        disabled={!canAddRow}
        onClick={onOpenImport}
        aria-label="Import data"
        title="Import data"
      >
        <IconPlus /> <span className="dbunk-primary-label">Import</span>
      </Button>
      {stagedChangeCount > 0 && onOpenReview ? (
        <Button
          size="sm"
          variant="secondary"
          onClick={onOpenReview}
          aria-label={`Review ${stagedChangeCount} staged changes`}
        >
          Review {stagedChangeCount}
        </Button>
      ) : null}
      <Button
        size="sm"
        variant="outline"
        className="border-danger/40 text-danger hover:bg-danger/10"
        disabled={!canDeleteSelected}
        aria-label="Delete selected"
        title="Delete selected"
        onClick={onDeleteSelected}
      >
        <IconTrash />{" "}
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
