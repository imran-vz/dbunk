import {
  IconArrowRight,
  IconCircleDot,
  IconExclamationCircle,
  IconKey,
  IconLink,
  IconMath,
  IconStar,
  IconTerminal2,
} from "@tabler/icons-react";
import {
  type ColumnDef,
  type ColumnFiltersState,
  flexRender,
  getCoreRowModel,
  getFilteredRowModel,
  getPaginationRowModel,
  type RowSelectionState,
  useReactTable,
  type VisibilityState,
} from "@tanstack/react-table";
import React, { useCallback, useEffect, useMemo, useState } from "react";

import type { ServerBrowseGridModel } from "@/components/data-grid/browse-model";
import {
  CELL_EDITORS,
  specializedCellKind,
} from "@/components/data-grid/cell-editors";
import {
  type AppliedFilter,
  DataGridToolbar,
  type ExportSettings,
} from "@/components/data-grid/toolbar";
import { Input } from "@/components/ui/input";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { ExportTable } from "@/lib/export";
import { cn } from "@/lib/utils";

/**
 * Per-column structural metadata for the data-grid header. Aligned
 * 1:1 with `columns`. Each flag drives a small icon next to the
 * column name; `dataType` and `derivationKind` flow into the
 * tooltip and a separate icon when relevant.
 */
export type ForeignKeyTarget = {
  schema: string;
  table: string;
  column: string;
};

export type ColumnHeaderMeta = {
  isPrimaryKey?: boolean;
  isForeignKey?: boolean;
  isIndexed?: boolean;
  isUnique?: boolean;
  notNull?: boolean;
  hasDefault?: boolean;
  dataType?: string;
  /** `"MATERIALIZED"` / `"ALIAS"` / `"EPHEMERAL"` for ClickHouse
   *  derived columns; `undefined` otherwise. */
  derivationKind?: string | null;
  /** Free-form description appended to the tooltip — e.g. the
   *  referenced FK target or the default expression. */
  description?: string;
  /** When present, FK cells render a hover arrow that calls
   *  `onFollowForeignKey` with this target + the cell value. */
  foreignKeyTarget?: ForeignKeyTarget;
};

const CELL_DISPLAY_CHARACTER_LIMIT = 50;

function formatCellDisplayValue(value: string) {
  return value.length > CELL_DISPLAY_CHARACTER_LIMIT
    ? value.slice(0, CELL_DISPLAY_CHARACTER_LIMIT)
    : value;
}

interface EditableCellProps {
  initialValue: string;
  rowIndex: number;
  columnIndex: number;
  columnName: string;
  columnType?: string;
  editValue?: string;
  onEdit?: (rowIndex: number, columnIndex: number, value: string) => void;
  /** When set, the cell renders a hover arrow that opens a
   *  drill-down to the referenced row(s). Caller wires the click
   *  through `onFollowForeignKey`. */
  foreignKeyTarget?: ForeignKeyTarget;
  onFollowForeignKey?: (
    rowIndex: number,
    target: ForeignKeyTarget,
    value: string,
  ) => void;
}

/**
 * Header cell rendering for a data column: name + a small strip of
 * icons keyed off `ColumnHeaderMeta`. The whole label carries a
 * `title` attribute so users get the full data-type + role
 * description on hover.
 *
 * Icon order is stable — left-to-right matches DBeaver-ish
 * convention: PK first, then FK, then indexed/unique, then
 * not-null, then default, then derivation. Absent flags simply
 * skip their slot.
 */
type MetaRow = {
  key: string;
  Icon: typeof IconKey;
  iconClass: string;
  label: string;
  /** Optional inline text rendered after the label in muted color. */
  detail?: string;
};

function buildMetaRows(meta: ColumnHeaderMeta): MetaRow[] {
  const rows: MetaRow[] = [];
  if (meta.isPrimaryKey) {
    rows.push({
      key: "pk",
      Icon: IconKey,
      iconClass: "text-warning",
      label: "Primary key",
    });
  }
  if (meta.isForeignKey) {
    rows.push({
      key: "fk",
      Icon: IconLink,
      iconClass: "text-primary",
      label: "Foreign key",
      detail: meta.description,
    });
  }
  if (meta.isUnique) {
    rows.push({
      key: "unique",
      Icon: IconStar,
      iconClass: "text-accent",
      label: "Unique index",
    });
  } else if (meta.isIndexed) {
    rows.push({
      key: "indexed",
      Icon: IconTerminal2,
      iconClass: "text-text-muted",
      label: "Indexed",
    });
  }
  if (meta.notNull) {
    rows.push({
      key: "notnull",
      Icon: IconExclamationCircle,
      iconClass: "text-rose-400",
      label: "NOT NULL",
    });
  }
  if (meta.hasDefault) {
    rows.push({
      key: "default",
      Icon: IconCircleDot,
      iconClass: "text-amber-400",
      label: "Has default",
    });
  }
  if (meta.derivationKind) {
    rows.push({
      key: "derived",
      Icon: IconMath,
      iconClass: "text-indigo-400",
      label: "Derived",
      detail: meta.derivationKind,
    });
  }
  return rows;
}

function ColumnHeaderLabel({
  name,
  meta,
  onSortClick,
}: {
  name: string;
  meta: ColumnHeaderMeta | undefined;
  onSortClick?: (event: React.MouseEvent<HTMLElement>) => void;
}) {
  if (!meta) {
    if (!onSortClick) return <span>{name}</span>;
    return (
      <button
        type="button"
        className="flex h-full w-full items-center px-2 text-left"
        onClick={onSortClick}
      >
        {name}
      </button>
    );
  }
  const rows = buildMetaRows(meta);

  const headerIcons = (
    <>
      {meta.isPrimaryKey ? (
        <IconKey
          className="size-3 shrink-0 text-warning"
          aria-label="primary key"
        />
      ) : null}
      {meta.isForeignKey ? (
        <IconLink
          className="size-3 shrink-0 text-primary"
          aria-label="foreign key"
        />
      ) : null}
      {meta.isUnique ? (
        <IconStar
          className="size-3 shrink-0 text-accent"
          aria-label="unique index"
        />
      ) : meta.isIndexed ? (
        <IconTerminal2
          className="size-3 shrink-0 text-text-muted"
          aria-label="indexed"
        />
      ) : null}
      {meta.notNull ? (
        <IconExclamationCircle
          className="size-3 shrink-0 text-rose-400"
          aria-label="NOT NULL"
        />
      ) : null}
      {meta.derivationKind ? (
        <IconMath
          className="size-3 shrink-0 text-indigo-400"
          aria-label={`${meta.derivationKind.toLowerCase()} column`}
        />
      ) : null}
    </>
  );

  const trigger = (
    <span className="flex items-center gap-1">
      {headerIcons}
      <span className="truncate">{name}</span>
    </span>
  );

  if (rows.length === 0 && !meta.dataType) return trigger;

  return (
    <Tooltip>
      <TooltipTrigger
        render={(props) => (
          <button
            type="button"
            {...props}
            className={cn(
              "bg-transparent p-0 text-left outline-none",
              onSortClick ? "h-full w-full cursor-pointer px-2" : "cursor-help",
            )}
            onClick={(event) => {
              props.onClick?.(event);
              onSortClick?.(event);
            }}
          >
            {trigger}
          </button>
        )}
      />
      <TooltipContent className="w-64 p-0">
        <div className="flex items-baseline gap-2 border-b border-border-subtle/60 px-3 py-2">
          <span className="truncate font-mono text-[0.75rem] font-semibold text-foreground">
            {name}
          </span>
          {meta.dataType ? (
            <span className="ml-auto shrink-0 rounded-sm bg-primary/15 px-1.5 py-0.5 font-mono text-[0.6rem] text-primary">
              {meta.dataType}
            </span>
          ) : null}
        </div>
        {rows.length > 0 ? (
          <ul className="space-y-1 px-3 py-2">
            {rows.map(({ key, Icon, iconClass, label, detail }) => (
              <li
                key={key}
                className="flex items-baseline gap-2 text-[0.65rem]"
              >
                <Icon
                  className={cn("size-3 shrink-0 self-center", iconClass)}
                  aria-hidden
                />
                <span className="text-foreground">{label}</span>
                {detail ? (
                  <span className="truncate font-mono text-text-muted">
                    {detail}
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}
      </TooltipContent>
    </Tooltip>
  );
}

function EditableCell({
  initialValue,
  rowIndex,
  columnIndex,
  columnName,
  columnType,
  editValue,
  onEdit,
  foreignKeyTarget,
  onFollowForeignKey,
}: EditableCellProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [overlayOpen, setOverlayOpen] = useState(false);
  const displayValue = editValue ?? initialValue;
  const previewValue = formatCellDisplayValue(displayValue);
  const isDirty = editValue !== undefined;
  const specializedKind = specializedCellKind(columnType);
  const SpecializedEditor = specializedKind
    ? CELL_EDITORS[specializedKind]
    : null;

  const onBlur = (e: React.FocusEvent<HTMLInputElement>) => {
    setIsEditing(false);
    if (e.target.value !== initialValue) {
      onEdit?.(rowIndex, columnIndex, e.target.value);
    } else if (isDirty) {
      // Allow clearing edit if value matches original
      onEdit?.(rowIndex, columnIndex, e.target.value);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.currentTarget.blur();
    }
    if (e.key === "Escape") {
      setIsEditing(false);
    }
  };

  const openEditor = () => {
    if (!onEdit) return;
    if (SpecializedEditor) {
      setOverlayOpen(true);
      return;
    }
    setIsEditing(true);
  };

  if (overlayOpen && SpecializedEditor) {
    return (
      <>
        <button
          type="button"
          className={cn(
            "h-full w-full truncate px-2 py-1 text-left text-muted-foreground",
            isDirty && "bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
          )}
          tabIndex={-1}
          title={displayValue}
        >
          {previewValue}
        </button>
        <SpecializedEditor
          initialValue={displayValue}
          columnName={columnName}
          onSave={(literal) => {
            setOverlayOpen(false);
            if (literal !== initialValue || isDirty) {
              onEdit?.(rowIndex, columnIndex, literal);
            }
          }}
          onCancel={() => setOverlayOpen(false)}
        />
      </>
    );
  }

  if (isEditing && onEdit) {
    return (
      <Input
        className="h-full w-full rounded-none border-0 bg-background px-2 py-0 text-xs shadow-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary"
        defaultValue={displayValue}
        onBlur={onBlur}
        onKeyDown={onKeyDown}
      />
    );
  }

  const canFollowFk =
    foreignKeyTarget !== undefined &&
    onFollowForeignKey !== undefined &&
    displayValue !== "" &&
    displayValue !== "NULL";

  return (
    <div className="group/cell relative flex h-full w-full items-center">
      <button
        type="button"
        className={cn(
          "h-full w-full truncate px-2 py-1 text-left text-muted-foreground group-hover:text-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary",
          isDirty && "bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
          !isEditing && onEdit && "cursor-pointer",
          !onEdit && "cursor-default",
          canFollowFk && "pr-6",
        )}
        onClick={openEditor}
        onKeyDown={(e) => {
          if (onEdit && (e.key === "Enter" || e.key === " ")) {
            openEditor();
          }
        }}
        tabIndex={onEdit ? 0 : -1}
        title={displayValue}
      >
        {previewValue}
      </button>
      {canFollowFk ? (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onFollowForeignKey?.(rowIndex, foreignKeyTarget, displayValue);
          }}
          className="absolute right-1 top-1/2 -translate-y-1/2 rounded-sm border border-border-subtle bg-surface-panel-elevated px-1 py-0.5 text-text-muted opacity-0 transition-opacity hover:text-foreground group-hover/cell:opacity-100 focus:opacity-100 focus:outline-none"
          aria-label={`Follow foreign key to ${foreignKeyTarget.schema}.${foreignKeyTarget.table}`}
          title={`Follow → ${foreignKeyTarget.schema}.${foreignKeyTarget.table}.${foreignKeyTarget.column}`}
        >
          <IconArrowRight className="size-3" />
        </button>
      ) : null}
    </div>
  );
}

export type TableViewMode = "data" | "structure";

export interface DataGridProps {
  data: string[][];
  columns: string[];
  /**
   * Optional per-column Postgres data types, aligned to `columns`.
   * When a column's type matches a specialized cell editor
   * (`json`/`jsonb`, `*[]`, `geometry`/`geography`), the grid opens
   * the registry editor instead of the inline single-line editor.
   * See ADR-0014.
   */
  columnTypes?: Array<string | undefined>;
  /**
   * Per-column structural metadata, aligned to `columns`. When
   * present, the grid renders icons in each header indicating the
   * column's role in the table (primary key, foreign key, indexed,
   * not-null, has-default, derived). Tooltips include the data
   * type. Absent metadata falls back to the bare column name.
   */
  columnMetadata?: Array<ColumnHeaderMeta | undefined>;
  /**
   * When provided, FK-marked cells render a hover arrow that calls
   * this callback with the row index, FK target, and cell value.
   * The row index lets the owning component anchor an inline
   * drill-down expansion at the right row.
   */
  onFollowForeignKey?: (
    rowIndex: number,
    target: ForeignKeyTarget,
    value: string,
  ) => void;
  edits?: Record<number, Record<number, string>>;
  onEdit?: (rowIndex: number, colIndex: number, value: string) => void;
  className?: string;
  onSave?: () => void;
  onDiscard?: () => void;
  onOpenSQL?: () => void;
  onRefresh?: () => void;
  hasEdits?: boolean;
  /**
   * When true, the grid suppresses cell editing entirely. Used when the
   * underlying table has no usable row identity (no PK, no non-null unique
   * index) — see `pickRowIdentity`.
   */
  readOnly?: boolean;
  /** Whether a commit is in flight; disables the Save button. */
  isSaving?: boolean;
  viewMode?: TableViewMode;
  onViewModeChange?: (mode: TableViewMode) => void;
  onToggleSidebar?: () => void;
  /**
   * Stem used for export filenames, e.g. "myconn-public-users-2026-05-09".
   * The grid appends `.csv` / `.json` and a "-selected" suffix as needed.
   * Defaults to "export" if omitted.
   */
  exportFilenameBase?: string;
  /**
   * Optional toolbar slot rendered between the Save/Discard cluster and
   * the filter/columns controls. Used by callers that want to extend the
   * grid's left toolbar with table-specific actions (e.g. Add row,
   * Delete selected) without forking the grid.
   */
  toolbarLeading?: React.ReactNode;
  /**
   * Controlled row-selection state. When provided, the parent owns the
   * selection map and is responsible for clearing it (e.g. after a
   * delete). Falls back to uncontrolled local state when omitted.
   */
  rowSelection?: RowSelectionState;
  onRowSelectionChange?: (selection: RowSelectionState) => void;
  onExportWholeTable?: (options: ExportSettings) => Promise<void>;
  onSaveExportTask?: (options: ExportSettings) => void;
  onRunSavedExportTask?: () => Promise<void>;
  hasSavedExportTask?: boolean;
  /**
   * Inline row expansion — when set, the grid renders an extra
   * `<tr>` directly under the row at `rowIndex` with `content`
   * spanning all data columns. Drives the Drizzle-style FK
   * drill-down preview without leaving the current view.
   */
  rowExpansion?: {
    rowIndex: number;
    content: React.ReactNode;
  } | null;
  serverBrowse?: ServerBrowseGridModel;
  onExpandGrid?: () => void;
  expanded?: boolean;
}

export function DataGrid({
  data,
  columns: columnNames,
  columnTypes,
  columnMetadata,
  onFollowForeignKey,
  rowExpansion,
  edits,
  onEdit,
  className,
  onSave,
  onDiscard,
  onOpenSQL,
  onRefresh,
  hasEdits,
  readOnly,
  isSaving,
  exportFilenameBase = "export",
  toolbarLeading,
  rowSelection: rowSelectionProp,
  onRowSelectionChange,
  onExportWholeTable,
  onSaveExportTask,
  onRunSavedExportTask,
  hasSavedExportTask,
  serverBrowse,
  onExpandGrid,
  expanded,
}: DataGridProps) {
  // When the grid is read-only we do not propagate the editor wiring to
  // cells. This both prevents `onEdit` from being called and makes the
  // visual affordance plain (no edit cursor, no focus ring on click).
  const effectiveOnEdit = readOnly ? undefined : onEdit;
  const [appliedFilters, setAppliedFilters] = useState<AppliedFilter[]>([]);
  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({});
  // Selection: when the parent passes `rowSelection` it owns the state; we
  // mirror it for tanstack-table. When uncontrolled, we keep the previous
  // local-state behavior for callers that don't care about lifting it.
  const [internalRowSelection, setInternalRowSelection] =
    useState<RowSelectionState>({});
  const rowSelection = rowSelectionProp ?? internalRowSelection;
  const setRowSelection = useCallback(
    (
      updater:
        | RowSelectionState
        | ((old: RowSelectionState) => RowSelectionState),
    ) => {
      const next =
        // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
        typeof updater === "function"
          ? // SAFETY: The value is constrained by the typed component or library contract at this boundary.
            // oxlint-disable-next-line anti-slop/require-safety-comment-for-type-assertion -- The value is handled at a typed library or domain boundary here.
            (updater as (old: RowSelectionState) => RowSelectionState)(
              rowSelection,
            )
          : updater;
      if (rowSelectionProp !== undefined) {
        onRowSelectionChange?.(next);
      } else {
        setInternalRowSelection(next);
        onRowSelectionChange?.(next);
      }
    },
    [rowSelection, rowSelectionProp, onRowSelectionChange],
  );

  useEffect(() => {
    setAppliedFilters((prev) => {
      const next = prev.filter((f) => columnNames.includes(f.column));
      return next.length === prev.length ? prev : next;
    });
  }, [columnNames]);

  const columnFilters = useMemo<ColumnFiltersState>(
    () => appliedFilters.map((f) => ({ id: f.column, value: f.value })),
    [appliedFilters],
  );

  const onApplyFilter = useCallback((filter: AppliedFilter) => {
    setAppliedFilters((prev) => {
      const idx = prev.findIndex((f) => f.column === filter.column);
      if (idx >= 0) {
        const copy = prev.slice();
        copy[idx] = filter;
        return copy;
      }
      return [...prev, filter];
    });
  }, []);

  const onRemoveFilter = useCallback((column: string) => {
    setAppliedFilters((prev) => prev.filter((f) => f.column !== column));
  }, []);

  const onClearFilters = useCallback(() => {
    setAppliedFilters([]);
  }, []);

  const columns = useMemo<ColumnDef<string[]>[]>(() => {
    const cols: ColumnDef<string[]>[] = [
      {
        id: "select",
        header: ({ table }) => (
          <div className="flex h-full items-center justify-center px-2 w-full">
            <input
              type="checkbox"
              className="size-3.5 rounded border-muted-foreground/40 bg-transparent accent-primary"
              checked={table.getIsAllPageRowsSelected()}
              onChange={table.getToggleAllPageRowsSelectedHandler()}
            />
          </div>
        ),
        cell: ({ row }) => (
          <div className="flex h-full items-center justify-center px-2">
            <input
              type="checkbox"
              className="size-3.5 rounded border-muted-foreground/40 bg-transparent accent-primary"
              checked={row.getIsSelected()}
              onChange={row.getToggleSelectedHandler()}
            />
          </div>
        ),
        enableSorting: false,
        enableHiding: false,
        size: 40,
      },
    ];

    columnNames.forEach((colName, index) => {
      const meta = columnMetadata?.[index];
      cols.push({
        accessorFn: (row) => row[index],
        id: colName,
        header: () => (
          <ColumnHeaderLabel
            name={colName}
            meta={meta}
            onSortClick={
              serverBrowse
                ? (event) => {
                    serverBrowse.onHeaderSort(colName, event.shiftKey);
                  }
                : undefined
            }
          />
        ),
        meta: { index },
        cell: (props) => (
          <EditableCell
            // SAFETY: The value is constrained by the typed component or library contract at this boundary.
            initialValue={props.getValue() as string}
            rowIndex={props.row.index}
            columnIndex={index}
            columnName={colName}
            columnType={columnTypes?.[index]}
            editValue={edits?.[props.row.index]?.[index]}
            onEdit={effectiveOnEdit}
            foreignKeyTarget={meta?.foreignKeyTarget}
            onFollowForeignKey={onFollowForeignKey}
          />
        ),
      });
    });

    return cols;
  }, [
    columnNames,
    columnTypes,
    columnMetadata,
    edits,
    effectiveOnEdit,
    onFollowForeignKey,
    serverBrowse,
  ]);

  const table = useReactTable({
    data,
    columns,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: serverBrowse ? undefined : getFilteredRowModel(),
    onColumnVisibilityChange: setColumnVisibility,
    onRowSelectionChange: setRowSelection,
    getPaginationRowModel: serverBrowse ? undefined : getPaginationRowModel(),
    state: {
      columnFilters: serverBrowse ? [] : columnFilters,
      columnVisibility,
      rowSelection,
    },
    initialState: {
      pagination: {
        pageSize: 50,
      },
    },
    manualFiltering: Boolean(serverBrowse),
    manualPagination: Boolean(serverBrowse),
  });

  const hasSelection = Object.keys(rowSelection).length > 0;

  // "All visible" = the rows currently in the filtered model. Column filters
  // applied via the Filters panel should be honored — that's what the user
  // sees in the grid. Pagination is not honored: exporting only the current
  // page of a multi-page result would be surprising.
  const buildExportTable = useCallback(
    (mode: "all" | "selected"): ExportTable => {
      const rowsModel =
        mode === "selected"
          ? table.getSelectedRowModel().rows
          : table.getFilteredRowModel().rows;
      const exportRows = rowsModel.map((row) =>
        columnNames.map((_col, colIndex) => row.original[colIndex] ?? null),
      );
      return { columns: columnNames, rows: exportRows };
    },
    [table, columnNames],
  );

  const visibleDataColumnCount = Math.max(
    1,
    table.getVisibleLeafColumns().filter((column) => column.id !== "select")
      .length,
  );
  const dataColumnWidth = `${100 / visibleDataColumnCount}%`;

  return (
    <div
      data-slot="data-grid"
      className={cn("flex h-full flex-col bg-surface-app", className)}
      aria-busy={serverBrowse?.loadStatus.state === "loading"}
    >
      <DataGridToolbar
        table={table}
        columnNames={columnNames}
        hasEdits={hasEdits}
        isSaving={isSaving}
        hasSelection={hasSelection}
        exportFilenameBase={exportFilenameBase || "export"}
        toolbarLeading={toolbarLeading}
        onSave={onSave}
        onDiscard={onDiscard}
        onRefresh={onRefresh}
        onOpenSQL={onOpenSQL}
        onExportWholeTable={onExportWholeTable}
        onSaveExportTask={onSaveExportTask}
        onRunSavedExportTask={onRunSavedExportTask}
        hasSavedExportTask={hasSavedExportTask}
        buildExportTable={buildExportTable}
        appliedFilters={appliedFilters}
        onApplyFilter={onApplyFilter}
        onRemoveFilter={onRemoveFilter}
        onClearFilters={onClearFilters}
        serverBrowse={serverBrowse}
        onExpandGrid={onExpandGrid}
        expanded={expanded}
      />

      <div
        data-slot="data-grid-scroll"
        className="flex-1 overflow-auto bg-surface-app"
      >
        {table.getRowModel().rows?.length ? (
          <table
            data-slot="data-grid-table"
            className={cn(
              "min-w-full border-separate border-spacing-0 text-left text-xs font-mono",
              serverBrowse?.loadStatus.state === "loading" && "opacity-60",
            )}
          >
            <thead className="sticky top-0 z-20">
              {table.getHeaderGroups().map((headerGroup) => (
                <tr key={headerGroup.id}>
                  {headerGroup.headers.map((header) => (
                    <th
                      key={header.id}
                      className={cn(
                        "sticky top-0 z-20 h-8 border-b border-r border-border-subtle bg-surface-panel-elevated px-0 align-middle font-medium text-text-muted last:border-r-0",
                        header.id === "select" && "sticky left-0 z-30 w-10",
                      )}
                      style={{
                        minWidth: header.id === "select" ? "40px" : "150px",
                        width:
                          header.id === "select" ? "40px" : dataColumnWidth,
                      }}
                    >
                      {header.isPlaceholder ? null : (
                        <div
                          className={cn(
                            "flex items-center gap-2",
                            header.id !== "select" && "px-2",
                          )}
                        >
                          {flexRender(
                            header.column.columnDef.header,
                            header.getContext(),
                          )}
                        </div>
                      )}
                    </th>
                  ))}
                </tr>
              ))}
            </thead>
            <tbody className="bg-surface-app">
              {table.getRowModel().rows.map((row) => {
                const visibleCells = row.getVisibleCells();
                const hasExpansion =
                  rowExpansion?.rowIndex === row.index &&
                  rowExpansion.content !== null;
                return (
                  <React.Fragment key={row.id}>
                    <tr
                      className={cn(
                        "group hover:bg-surface-row-hover",
                        row.getIsSelected() &&
                          "bg-accent-overlay text-foreground",
                      )}
                    >
                      {visibleCells.map((cell) => (
                        <td
                          key={cell.id}
                          className={cn(
                            "h-8 border-b border-r border-border-subtle p-0 align-middle last:border-r-0",
                            cell.column.id === "select" &&
                              "sticky left-0 z-10 w-10 bg-surface-app group-hover:bg-surface-row-hover",
                            cell.column.id === "select" &&
                              row.getIsSelected() &&
                              "bg-primary/10",
                          )}
                          style={{
                            minWidth:
                              cell.column.id === "select" ? "40px" : "150px",
                            width:
                              cell.column.id === "select"
                                ? "40px"
                                : dataColumnWidth,
                          }}
                        >
                          {flexRender(
                            cell.column.columnDef.cell,
                            cell.getContext(),
                          )}
                        </td>
                      ))}
                    </tr>
                    {hasExpansion ? (
                      <tr key={`${row.id}-expansion`}>
                        <td
                          colSpan={visibleCells.length}
                          className="border-b border-border-subtle bg-surface-panel/40 p-0"
                        >
                          {rowExpansion?.content}
                        </td>
                      </tr>
                    ) : null}
                  </React.Fragment>
                );
              })}
            </tbody>
          </table>
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-muted-foreground">
            <div className="text-xs">No data available</div>
          </div>
        )}
      </div>
    </div>
  );
}
