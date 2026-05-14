import {
  IconArrowsMaximize,
  IconArrowsSort,
  IconColumns,
  IconDeviceFloppy,
  IconDownload,
  IconFilter,
  IconRefresh,
  IconSearch,
  IconX,
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
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  CELL_EDITORS,
  specializedCellKind,
} from "@/components/data-grid/cell-editors";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { downloadBlob, downloadFile } from "@/lib/download";
import {
  type ExportCompression,
  type ExportEncoding,
  type ExportFormat,
  type ExportTable,
  prepareExport,
  prepareExportBlob,
} from "@/lib/export";
import { cn } from "@/lib/utils";

const FILTER_OPERATORS = [
  { label: "equals", symbol: "=" },
  { label: "not equals", symbol: "<>" },
  { label: "greater", symbol: ">" },
  { label: "greater or equals", symbol: ">=" },
  { label: "less", symbol: "<" },
  { label: "less or equals", symbol: "<=" },
  { label: "like", symbol: "LIKE" },
  { label: "ilike", symbol: "ILIKE" },
  { label: "not like", symbol: "NOT LIKE" },
  { label: "in", symbol: "IN" },
  { label: "is null", symbol: "IS NULL" },
  { label: "is not null", symbol: "IS NOT NULL" },
] as const;

const OPERATOR_SYMBOL: Record<string, string> = Object.fromEntries(
  FILTER_OPERATORS.map((op) => [op.label, op.symbol]),
);

const CELL_DISPLAY_CHARACTER_LIMIT = 50;

function formatCellDisplayValue(value: string) {
  return value.length > CELL_DISPLAY_CHARACTER_LIMIT
    ? value.slice(0, CELL_DISPLAY_CHARACTER_LIMIT)
    : value;
}

type AppliedFilter = {
  column: string;
  operator: string;
  value: string;
};

interface EditableCellProps {
  initialValue: string;
  rowIndex: number;
  columnIndex: number;
  columnName: string;
  columnType?: string;
  editValue?: string;
  onEdit?: (rowIndex: number, columnIndex: number, value: string) => void;
}

function EditableCell({
  initialValue,
  rowIndex,
  columnIndex,
  columnName,
  columnType,
  editValue,
  onEdit,
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
        autoFocus
        className="h-full w-full rounded-none border-0 bg-background px-2 py-0 text-xs shadow-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary"
        defaultValue={displayValue}
        onBlur={onBlur}
        onKeyDown={onKeyDown}
      />
    );
  }

  return (
    <button
      type="button"
      className={cn(
        "h-full w-full truncate px-2 py-1 text-left text-muted-foreground group-hover:text-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary",
        isDirty && "bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
        !isEditing && onEdit && "cursor-pointer",
        !onEdit && "cursor-default",
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

export function DataGrid({
  data,
  columns: columnNames,
  columnTypes,
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
        typeof updater === "function"
          ? (updater as (old: RowSelectionState) => RowSelectionState)(
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
  const [showFilters, setShowFilters] = useState(false);
  const [columnSearch, setColumnSearch] = useState("");
  // Draft is intentionally separate from appliedFilters: editing the draft
  // (column, operator, value) must not change what's filtering the grid
  // until the user explicitly clicks Apply or hits Enter.
  const [draftColumn, setDraftColumn] = useState<string>(columnNames[0] ?? "");
  const [draftOperator, setDraftOperator] = useState("equals");
  const [draftValue, setDraftValue] = useState("");
  const [exportFormat, setExportFormat] = useState<ExportFormat>("csv");
  const [exportEncoding, setExportEncoding] = useState<ExportEncoding>("utf-8");
  const [exportCompression, setExportCompression] =
    useState<ExportCompression>("none");
  const [exportNullAs, setExportNullAs] = useState("");

  useEffect(() => {
    if (columnNames.length === 0) return;
    if (!columnNames.includes(draftColumn)) {
      setDraftColumn(columnNames[0]);
    }
  }, [columnNames, draftColumn]);

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

  const canApplyDraft = draftColumn !== "" && draftValue.trim() !== "";

  const applyDraft = useCallback(() => {
    if (!canApplyDraft) return;
    setAppliedFilters((prev) => {
      const next: AppliedFilter = {
        column: draftColumn,
        operator: draftOperator,
        value: draftValue,
      };
      const idx = prev.findIndex((f) => f.column === draftColumn);
      if (idx >= 0) {
        const copy = prev.slice();
        copy[idx] = next;
        return copy;
      }
      return [...prev, next];
    });
  }, [canApplyDraft, draftColumn, draftOperator, draftValue]);

  const removeFilter = useCallback((column: string) => {
    setAppliedFilters((prev) => prev.filter((f) => f.column !== column));
  }, []);

  const clearAllFilters = useCallback(() => {
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
      cols.push({
        accessorFn: (row) => row[index],
        id: colName,
        header: colName,
        meta: { index },
        cell: (props) => (
          <EditableCell
            initialValue={props.getValue() as string}
            rowIndex={props.row.index}
            columnIndex={index}
            columnName={colName}
            columnType={columnTypes?.[index]}
            editValue={edits?.[props.row.index]?.[index]}
            onEdit={effectiveOnEdit}
          />
        ),
      });
    });

    return cols;
  }, [columnNames, columnTypes, edits, effectiveOnEdit]);

  const table = useReactTable({
    data,
    columns,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: getFilteredRowModel(),
    onColumnVisibilityChange: setColumnVisibility,
    onRowSelectionChange: setRowSelection,
    getPaginationRowModel: getPaginationRowModel(),
    state: {
      columnFilters,
      columnVisibility,
      rowSelection,
    },
    initialState: {
      pagination: {
        pageSize: 50,
      },
    },
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

  const handleExport = useCallback(
    async (mode: "all" | "selected", format: ExportFormat) => {
      const exportTable = buildExportTable(mode);
      const filenameBase = `${exportFilenameBase || "export"}${
        mode === "selected" ? "-selected" : ""
      }`;
      const prepared = prepareExport(exportTable, {
        format,
        filenameBase,
        encoding: exportEncoding,
        compression: exportCompression,
        nullAs: exportNullAs,
      });
      if (exportCompression === "gzip") {
        const { filename, blob } = await prepareExportBlob(exportTable, {
          format,
          filenameBase,
          encoding: exportEncoding,
          compression: exportCompression,
          nullAs: exportNullAs,
        });
        downloadBlob(filename, blob);
        return;
      }
      downloadFile(prepared.filename, prepared.mime, prepared.content);
    },
    [
      buildExportTable,
      exportCompression,
      exportEncoding,
      exportFilenameBase,
      exportNullAs,
    ],
  );

  const exportSettings = {
    format: exportFormat,
    encoding: exportEncoding,
    compression: exportCompression,
    nullAs: exportNullAs,
  };
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
    >
      <div
        data-slot="data-grid-toolbar"
        className="flex min-h-10 shrink-0 flex-wrap items-center justify-between gap-x-2 gap-y-1.5 overflow-x-auto border-b border-border-subtle bg-surface-window px-3 py-1.5"
      >
        <div className="flex flex-wrap items-center gap-1.5">
          {hasEdits ? (
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="ghost"
                onClick={onDiscard}
                disabled={isSaving}
              >
                <IconX className="mr-1 size-3.5" />{" "}
                <span className="dbunk-optional-label">Discard</span>
              </Button>
              <Button size="sm" onClick={onSave} disabled={isSaving}>
                <IconDeviceFloppy className="mr-1 size-3.5" />{" "}
                <span className="dbunk-primary-label">
                  {isSaving ? "Saving…" : "Save changes"}
                </span>
              </Button>
              <div className="mx-1 h-5 w-px bg-border-subtle" />
            </div>
          ) : null}

          <Button
            variant={showFilters ? "secondary" : "outline"}
            size="sm"
            className={cn(
              "gap-1.5 border-border-subtle bg-surface-panel",
              showFilters && "bg-primary/10 text-primary",
            )}
            onClick={() => setShowFilters(!showFilters)}
            aria-label="Filter"
            title="Filter"
          >
            <IconFilter className="size-3.5" />
            <span className="dbunk-optional-label">Filter</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            className="gap-1.5 border-border-subtle bg-surface-panel"
            aria-label="Sort"
            title="Sort"
          >
            <IconArrowsSort className="size-3.5" />
            <span className="dbunk-optional-label">Sort</span>
          </Button>

          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="Columns"
              title="Columns"
              className={cn(
                buttonVariants({ variant: "outline", size: "sm" }),
                "gap-1.5 border-border-subtle bg-surface-panel",
              )}
            >
              <IconColumns className="size-3.5" />
              <span className="dbunk-optional-label">Columns</span>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              <DropdownMenuGroup>
                <div className="flex items-center justify-between px-2 py-1.5">
                  <span className="text-xs font-semibold">Toggle columns</span>
                  <button
                    type="button"
                    className="text-xs text-muted-foreground hover:text-foreground"
                    onClick={(e) => {
                      e.preventDefault();
                      table.toggleAllColumnsVisible(false);
                    }}
                  >
                    Deselect all
                  </button>
                </div>
                <div className="px-2 pb-2">
                  <div className="relative">
                    <IconSearch className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
                    <Input
                      placeholder="Search..."
                      className="h-7 pl-8 text-xs"
                      value={columnSearch}
                      onChange={(e) => setColumnSearch(e.target.value)}
                    />
                  </div>
                </div>
                <DropdownMenuSeparator />
                <div className="max-h-75 overflow-auto">
                  {table
                    .getAllColumns()
                    .filter(
                      (column) =>
                        typeof column.accessorFn !== "undefined" &&
                        column.getCanHide() &&
                        column.id
                          .toLowerCase()
                          .includes(columnSearch.toLowerCase()),
                    )
                    .map((column) => {
                      return (
                        <DropdownMenuCheckboxItem
                          key={column.id}
                          className="capitalize"
                          checked={column.getIsVisible()}
                          onCheckedChange={(value) =>
                            column.toggleVisibility(!!value)
                          }
                        >
                          {column.id}
                        </DropdownMenuCheckboxItem>
                      );
                    })}
                </div>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>

          <Button
            variant="outline"
            size="sm"
            className="gap-1.5 border-border-subtle bg-surface-panel"
            onClick={onRefresh}
            disabled={!onRefresh}
            aria-label="Refresh"
            title="Refresh"
          >
            <IconRefresh className="size-3.5" />
            <span className="dbunk-optional-label">Refresh</span>
          </Button>

          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="Export"
              title="Export"
              className={cn(
                buttonVariants({ variant: "outline", size: "sm" }),
                "gap-1.5 border-border-subtle bg-surface-panel",
              )}
            >
              <IconDownload className="size-3.5" />
              <span className="dbunk-optional-label">Export</span>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-56">
              <DropdownMenuGroup>
                <div className="px-2 py-1.5 text-xs font-semibold">
                  Current result rows
                </div>
                {(
                  [
                    "json",
                    "csv",
                    "sql",
                    "html",
                    "markdown",
                    "txt",
                    "xlsx",
                  ] as const
                ).map((format) => (
                  <DropdownMenuItem
                    key={`all-${format}`}
                    onClick={() => {
                      void handleExport("all", format);
                    }}
                  >
                    <IconDownload className="mr-2 size-3.5" />
                    Export all to .{format === "markdown" ? "md" : format}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <div className="px-2 py-1.5 text-xs font-semibold">
                  Selected rows
                </div>
                {(
                  [
                    "json",
                    "csv",
                    "sql",
                    "html",
                    "markdown",
                    "txt",
                    "xlsx",
                  ] as const
                ).map((format) => (
                  <DropdownMenuItem
                    key={`selected-${format}`}
                    disabled={!hasSelection}
                    onClick={() => {
                      void handleExport("selected", format);
                    }}
                  >
                    <IconDownload className="mr-2 size-3.5" />
                    Export selected to .{format === "markdown" ? "md" : format}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuGroup>
              {onExportWholeTable ? (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuGroup>
                    <div className="px-2 py-1.5 text-xs font-semibold">
                      Whole table
                    </div>
                    <DropdownMenuItem
                      onClick={() => {
                        void onExportWholeTable(exportSettings);
                      }}
                    >
                      <IconDownload className="mr-2 size-3.5" />
                      Export whole table
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      disabled={!onSaveExportTask}
                      onClick={() => onSaveExportTask?.(exportSettings)}
                    >
                      <IconDeviceFloppy className="mr-2 size-3.5" />
                      Save export task
                    </DropdownMenuItem>
                    <DropdownMenuItem
                      disabled={!hasSavedExportTask || !onRunSavedExportTask}
                      onClick={() => {
                        void onRunSavedExportTask?.();
                      }}
                    >
                      <IconRefresh className="mr-2 size-3.5" />
                      Run saved export task
                    </DropdownMenuItem>
                  </DropdownMenuGroup>
                </>
              ) : null}
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <div className="px-2 py-1.5 text-xs font-semibold">
                  Export options
                </div>
                <DropdownMenuCheckboxItem
                  checked={exportEncoding === "utf-16le"}
                  onCheckedChange={(checked) =>
                    setExportEncoding(checked ? "utf-16le" : "utf-8")
                  }
                >
                  UTF-16LE encoding
                </DropdownMenuCheckboxItem>
                <DropdownMenuCheckboxItem
                  checked={exportCompression === "gzip"}
                  onCheckedChange={(checked) =>
                    setExportCompression(checked ? "gzip" : "none")
                  }
                >
                  Gzip compression
                </DropdownMenuCheckboxItem>
                <div className="px-2 py-1.5">
                  <Input
                    className="h-7 text-xs"
                    placeholder="NULL token"
                    value={exportNullAs}
                    onChange={(event) => setExportNullAs(event.target.value)}
                  />
                </div>
                <div className="px-2 py-1.5">
                  <Select
                    value={exportFormat}
                    onValueChange={(value) =>
                      setExportFormat((value as ExportFormat) ?? "csv")
                    }
                  >
                    <SelectTrigger className="h-7 text-xs">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(
                        [
                          "csv",
                          "json",
                          "sql",
                          "html",
                          "markdown",
                          "txt",
                          "xlsx",
                        ] as const
                      ).map((format) => (
                        <SelectItem key={format} value={format}>
                          {format}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>

          {toolbarLeading ? (
            <>
              <div className="mx-0.5 h-5 w-px bg-border-subtle" />
              {toolbarLeading}
            </>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <Select defaultValue={String(table.getState().pagination.pageSize)}>
            <SelectTrigger className="h-6 w-24 border-border-subtle bg-surface-panel text-xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {[10, 25, 50, 100].map((size) => (
                <SelectItem key={size} value={String(size)}>
                  {size} rows
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            variant="ghost"
            size="icon"
            aria-label="Expand grid"
            className="rounded-sm border border-border-subtle bg-surface-panel"
          >
            <IconArrowsMaximize className="size-3.5" />
          </Button>
        </div>
      </div>

      {showFilters && (
        <div className="flex min-h-10 flex-wrap items-center gap-1.5 border-b border-border-subtle bg-surface-window px-3 py-1.5">
          <Button
            variant="ghost"
            size="icon"
            className="shrink-0 text-muted-foreground"
            onClick={() => setShowFilters(false)}
          >
            <IconX className="size-3.5" />
          </Button>

          {appliedFilters.length > 0 && (
            <div className="flex flex-wrap items-center gap-1">
              {appliedFilters.map((f) => (
                <div
                  key={f.column}
                  className="flex h-6 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-xs"
                >
                  <span className="font-medium">{f.column}</span>
                  <span className="font-mono text-[10px] text-muted-foreground">
                    {OPERATOR_SYMBOL[f.operator] ?? f.operator}
                  </span>
                  <span className="max-w-32 truncate text-muted-foreground">
                    {f.value}
                  </span>
                  <button
                    type="button"
                    className="ml-0.5 rounded-sm text-muted-foreground hover:text-foreground"
                    onClick={() => removeFilter(f.column)}
                    aria-label={`Remove filter on ${f.column}`}
                  >
                    <IconX className="size-3" />
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="flex flex-wrap items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel p-1">
            <div className="flex items-center gap-1.5 rounded-sm bg-surface-app px-1.5">
              <span className="text-xs font-medium text-muted-foreground">
                where
              </span>
              <Select
                value={draftColumn}
                onValueChange={(val) => setDraftColumn(val ?? "")}
              >
                <SelectTrigger className="h-6 w-auto min-w-24 border-none bg-transparent px-1.5 text-xs shadow-none hover:bg-muted/50 focus:ring-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {columnNames.map((col) => (
                    <SelectItem key={col} value={col}>
                      {col}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="flex items-center gap-1.5 rounded-sm bg-surface-app px-1.5">
              <Select
                value={draftOperator}
                onValueChange={(val) => setDraftOperator(val ?? "equals")}
              >
                <SelectTrigger className="h-6 w-auto min-w-28 border-none bg-transparent px-1.5 text-xs shadow-none hover:bg-muted/50 focus:ring-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {FILTER_OPERATORS.map((op) => (
                    <SelectItem key={op.label} value={op.label}>
                      <div className="flex w-full items-center justify-between gap-4">
                        <span>{op.label}</span>
                        <span className="font-mono text-[10px] text-muted-foreground bg-muted/50 px-1 rounded-xs">
                          {op.symbol}
                        </span>
                      </div>
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <Input
              className="h-6 w-36 border-none bg-surface-app px-2 text-xs shadow-none placeholder:text-muted-foreground/50 focus-visible:ring-0 sm:w-56"
              placeholder="value"
              value={draftValue}
              onChange={(event) => setDraftValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  applyDraft();
                }
              }}
            />

            <Button
              size="sm"
              variant="secondary"
              onClick={applyDraft}
              disabled={!canApplyDraft}
            >
              Apply
            </Button>
          </div>

          <div className="ml-auto flex items-center gap-1.5">
            <Button
              variant="secondary"
              size="sm"
              className="hidden bg-muted/50 text-muted-foreground hover:bg-muted sm:inline-flex"
              onClick={onOpenSQL}
              disabled={!onOpenSQL}
            >
              Open in SQL
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground hover:text-foreground"
              onClick={clearAllFilters}
              disabled={appliedFilters.length === 0}
            >
              Clear filters
            </Button>
          </div>
        </div>
      )}

      <div
        data-slot="data-grid-scroll"
        className="flex-1 overflow-auto bg-surface-app"
      >
        {table.getRowModel().rows?.length ? (
          <table
            data-slot="data-grid-table"
            className="min-w-full border-separate border-spacing-0 text-left text-xs font-mono"
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
              {table.getRowModel().rows.map((row) => (
                <tr
                  key={row.id}
                  className={cn(
                    "group hover:bg-surface-row-hover",
                    row.getIsSelected() && "bg-accent-overlay text-foreground",
                  )}
                >
                  {row.getVisibleCells().map((cell) => (
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
              ))}
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
