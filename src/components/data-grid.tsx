import {
  IconChevronLeft,
  IconChevronRight,
  IconColumns,
  IconDeviceFloppy,
  IconDots,
  IconDownload,
  IconFilter,
  IconLayoutSidebar,
  IconRefresh,
  IconSearch,
  IconStack2,
  IconTable,
  IconTerminal2,
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
import { downloadFile } from "@/lib/download";
import { type ExportTable, toCsv, toJson } from "@/lib/export";
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

type AppliedFilter = {
  column: string;
  operator: string;
  value: string;
};

interface EditableCellProps {
  initialValue: string;
  rowIndex: number;
  columnIndex: number;
  editValue?: string;
  onEdit?: (rowIndex: number, columnIndex: number, value: string) => void;
}

function EditableCell({
  initialValue,
  rowIndex,
  columnIndex,
  editValue,
  onEdit,
}: EditableCellProps) {
  const [isEditing, setIsEditing] = useState(false);
  const displayValue = editValue ?? initialValue;
  const isDirty = editValue !== undefined;

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
        "h-full w-full truncate px-3 py-2 text-left text-muted-foreground group-hover:text-foreground outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-primary",
        isDirty && "bg-yellow-500/10 text-yellow-600 dark:text-yellow-400",
        !isEditing && onEdit && "cursor-pointer",
        !onEdit && "cursor-default",
      )}
      onClick={() => onEdit && setIsEditing(true)}
      onKeyDown={(e) => {
        if (onEdit && (e.key === "Enter" || e.key === " ")) {
          setIsEditing(true);
        }
      }}
      tabIndex={onEdit ? 0 : -1}
    >
      {displayValue}
    </button>
  );
}

export type TableViewMode = "data" | "structure";

export interface DataGridProps {
  data: string[][];
  columns: string[];
  edits?: Record<number, Record<number, string>>;
  onEdit?: (rowIndex: number, colIndex: number, value: string) => void;
  className?: string;
  onSave?: () => void;
  onDiscard?: () => void;
  onOpenSQL?: () => void;
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
}

export function DataGrid({
  data,
  columns: columnNames,
  edits,
  onEdit,
  className,
  onSave,
  onDiscard,
  onOpenSQL,
  hasEdits,
  readOnly,
  isSaving,
  viewMode = "data",
  onViewModeChange,
  onToggleSidebar,
  exportFilenameBase = "export",
  toolbarLeading,
  rowSelection: rowSelectionProp,
  onRowSelectionChange,
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
            editValue={edits?.[props.row.index]?.[index]}
            onEdit={effectiveOnEdit}
          />
        ),
      });
    });

    return cols;
  }, [columnNames, edits, effectiveOnEdit]);

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

  const filenameFor = useCallback(
    (mode: "all" | "selected", ext: "csv" | "json") => {
      const base = exportFilenameBase || "export";
      const suffix = mode === "selected" ? "-selected" : "";
      return `${base}${suffix}.${ext}`;
    },
    [exportFilenameBase],
  );

  const handleExportCsv = useCallback(
    (mode: "all" | "selected") => {
      const exportTable = buildExportTable(mode);
      downloadFile(
        filenameFor(mode, "csv"),
        "text/csv;charset=utf-8",
        toCsv(exportTable),
      );
    },
    [buildExportTable, filenameFor],
  );

  const handleExportJson = useCallback(
    (mode: "all" | "selected") => {
      const exportTable = buildExportTable(mode);
      downloadFile(
        filenameFor(mode, "json"),
        "application/json;charset=utf-8",
        toJson(exportTable, { pretty: true }),
      );
    },
    [buildExportTable, filenameFor],
  );

  return (
    <div className={cn("flex h-full flex-col bg-background", className)}>
      <div className="flex min-h-14 shrink-0 flex-wrap items-center justify-between gap-x-4 gap-y-2 border-b px-4 py-2">
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <div className="flex items-center rounded-md border bg-muted/20 p-0.5">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 rounded-sm"
              onClick={onToggleSidebar}
            >
              <IconLayoutSidebar className="size-4 text-muted-foreground" />
            </Button>
            <Button
              variant={viewMode === "data" ? "secondary" : "ghost"}
              size="icon"
              className="h-7 w-7 rounded-sm"
              onClick={() => onViewModeChange?.("data")}
            >
              <IconTable
                className={cn(
                  "size-4",
                  viewMode === "data"
                    ? "text-foreground"
                    : "text-muted-foreground",
                )}
              />
            </Button>
            <Button
              variant={viewMode === "structure" ? "secondary" : "ghost"}
              size="icon"
              className="h-7 w-7 rounded-sm"
              onClick={() => onViewModeChange?.("structure")}
            >
              <IconStack2
                className={cn(
                  "size-4",
                  viewMode === "structure"
                    ? "text-foreground"
                    : "text-muted-foreground",
                )}
              />
            </Button>
          </div>

          {hasEdits ? (
            <div className="flex items-center gap-2">
              <Button
                size="sm"
                variant="ghost"
                className="h-9 px-2 text-xs"
                onClick={onDiscard}
                disabled={isSaving}
              >
                <IconX className="mr-1 size-3.5" /> Discard
              </Button>
              <Button
                size="sm"
                className="h-9 px-2 text-xs"
                onClick={onSave}
                disabled={isSaving}
              >
                <IconDeviceFloppy className="mr-1 size-3.5" />{" "}
                {isSaving ? "Saving…" : "Save changes"}
              </Button>
              <div className="mx-2 h-6 w-px bg-border" />
            </div>
          ) : (
            onOpenSQL && (
              <Button
                variant="ghost"
                size="sm"
                className="h-9 gap-2 px-2 text-muted-foreground hover:text-foreground"
                onClick={onOpenSQL}
              >
                <IconTerminal2 className="size-4" />
                SQL
              </Button>
            )
          )}

          {toolbarLeading}

          <div className="flex items-center gap-2">
            <Button
              variant={showFilters ? "secondary" : "outline"}
              size="sm"
              className={cn(
                "h-9 gap-2 border-dashed",
                showFilters && "border-solid",
              )}
              onClick={() => setShowFilters(!showFilters)}
            >
              <IconFilter className="size-3.5" />
              Filters
            </Button>

            <DropdownMenu>
              <DropdownMenuTrigger
                className={cn(
                  buttonVariants({ variant: "outline", size: "sm" }),
                  "h-9 gap-2 border-dashed",
                )}
              >
                <IconColumns className="size-3.5" />
                Columns
              </DropdownMenuTrigger>
              <DropdownMenuContent align="start" className="w-56">
                <DropdownMenuGroup>
                  <div className="flex items-center justify-between px-2 py-1.5">
                    <span className="text-xs font-semibold">
                      Toggle columns
                    </span>
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
                        className="h-8 pl-8 text-xs"
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
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <div className="hidden items-center gap-1 text-xs text-muted-foreground md:flex">
            <span className="tabular-nums whitespace-nowrap">
              {table.getFilteredRowModel().rows.length} rows • 137ms
            </span>
          </div>
          <div className="flex items-center rounded-md border bg-muted/20 p-0.5">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 rounded-sm"
              onClick={() => table.previousPage()}
              disabled={!table.getCanPreviousPage()}
            >
              <IconChevronLeft className="size-3.5" />
            </Button>
            <div className="hidden h-7 min-w-8 items-center justify-center border-x px-2 text-xs font-medium tabular-nums sm:flex">
              {table.getState().pagination.pageSize}
            </div>
            <div className="flex h-7 min-w-8 items-center justify-center border-x px-2 text-xs font-medium tabular-nums sm:border-l-0">
              {table.getState().pagination.pageIndex + 1}
            </div>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 rounded-sm"
              onClick={() => table.nextPage()}
              disabled={!table.getCanNextPage()}
            >
              <IconChevronRight className="size-3.5" />
            </Button>
          </div>

          <Button
            variant="ghost"
            size="icon"
            className="hidden h-8 w-8 rounded-md border bg-muted/20 md:inline-flex"
          >
            <IconRefresh className="size-3.5" />
          </Button>

          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-8 w-8 rounded-md border bg-muted/20"
                >
                  <IconDots className="size-3.5" />
                </Button>
              }
            />
            <DropdownMenuContent align="end" className="w-56">
              <DropdownMenuGroup>
                <DropdownMenuItem>
                  <IconRefresh className="mr-2 size-3.5" />
                  Refresh rows
                </DropdownMenuItem>
                <DropdownMenuItem>
                  <IconRefresh className="mr-2 size-3.5" />
                  Refresh schema
                </DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
              <DropdownMenuGroup>
                <DropdownMenuItem onClick={() => handleExportJson("all")}>
                  <IconDownload className="mr-2 size-3.5" />
                  Export all to .json
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => handleExportCsv("all")}>
                  <IconDownload className="mr-2 size-3.5" />
                  Export all to .csv
                </DropdownMenuItem>
                <DropdownMenuItem disabled>
                  <IconDownload className="mr-2 size-3.5" />
                  Export all to .xlsx
                </DropdownMenuItem>
              </DropdownMenuGroup>
              <DropdownMenuGroup>
                <DropdownMenuItem
                  disabled={!hasSelection}
                  onClick={() => handleExportJson("selected")}
                >
                  <IconDownload className="mr-2 size-3.5" />
                  Export selected to .json
                </DropdownMenuItem>
                <DropdownMenuItem
                  disabled={!hasSelection}
                  onClick={() => handleExportCsv("selected")}
                >
                  <IconDownload className="mr-2 size-3.5" />
                  Export selected to .csv
                </DropdownMenuItem>
                <DropdownMenuItem disabled>
                  <IconDownload className="mr-2 size-3.5" />
                  Export selected to .xlsx
                </DropdownMenuItem>
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {showFilters && (
        <div className="flex min-h-12 flex-wrap items-center gap-2 border-b bg-muted/10 px-4 py-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-6 w-6 shrink-0 text-muted-foreground"
            onClick={() => setShowFilters(false)}
          >
            <IconX className="size-3.5" />
          </Button>

          {appliedFilters.length > 0 && (
            <div className="flex flex-wrap items-center gap-1.5">
              {appliedFilters.map((f) => (
                <div
                  key={f.column}
                  className="flex h-7 items-center gap-1.5 rounded-md border bg-background px-2 text-xs shadow-sm"
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

          <div className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/20 p-1">
            <div className="flex items-center gap-2 rounded-sm bg-background px-2 shadow-sm">
              <span className="text-xs font-medium text-muted-foreground">
                where
              </span>
              <Select
                value={draftColumn}
                onValueChange={(val) => setDraftColumn(val ?? "")}
              >
                <SelectTrigger className="h-7 w-auto min-w-25 border-none bg-transparent px-2 text-xs shadow-none hover:bg-muted/50 focus:ring-0">
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

            <div className="flex items-center gap-2 rounded-sm bg-background px-2 shadow-sm">
              <Select
                value={draftOperator}
                onValueChange={(val) => setDraftOperator(val ?? "equals")}
              >
                <SelectTrigger className="h-7 w-auto min-w-30 border-none bg-transparent px-2 text-xs shadow-none hover:bg-muted/50 focus:ring-0">
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
              className="h-7 w-40 border-none bg-muted/20 px-2 text-xs shadow-none focus-visible:ring-0 placeholder:text-muted-foreground/50 sm:w-60"
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
              className="h-7 px-3 text-xs"
              onClick={applyDraft}
              disabled={!canApplyDraft}
            >
              Apply
            </Button>
          </div>

          <div className="ml-auto flex items-center gap-2">
            <Button
              variant="secondary"
              size="sm"
              className="hidden h-7 text-xs bg-muted/50 hover:bg-muted text-muted-foreground sm:inline-flex"
              onClick={onOpenSQL}
              disabled={!onOpenSQL}
            >
              Open in SQL
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 text-xs text-muted-foreground hover:text-foreground"
              onClick={clearAllFilters}
              disabled={appliedFilters.length === 0}
            >
              Clear filters
            </Button>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-auto bg-background">
        {table.getRowModel().rows?.length ? (
          <table className="min-w-full border-separate border-spacing-0 text-left text-xs font-mono">
            <thead className="sticky top-0 z-20">
              {table.getHeaderGroups().map((headerGroup) => (
                <tr key={headerGroup.id}>
                  {headerGroup.headers.map((header) => (
                    <th
                      key={header.id}
                      className={cn(
                        "sticky top-0 z-20 h-9 border-b border-r bg-muted px-0 align-middle font-medium text-muted-foreground last:border-r-0",
                        header.id === "select" && "sticky left-0 z-30 w-10",
                      )}
                      style={{
                        minWidth: header.id === "select" ? "40px" : "150px",
                        maxWidth: header.id === "select" ? "40px" : "300px",
                      }}
                    >
                      {header.isPlaceholder ? null : (
                        <div
                          className={cn(
                            "flex items-center gap-2",
                            header.id !== "select" && "px-3",
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
            <tbody className="bg-background">
              {table.getRowModel().rows.map((row) => (
                <tr
                  key={row.id}
                  className={cn(
                    "group hover:bg-muted/5",
                    row.getIsSelected() && "bg-muted/10",
                  )}
                >
                  {row.getVisibleCells().map((cell) => (
                    <td
                      key={cell.id}
                      className={cn(
                        "h-9 border-b border-r p-0 align-middle last:border-r-0",
                        cell.column.id === "select" &&
                          "sticky left-0 z-10 w-10 bg-background group-hover:bg-muted/5",
                        cell.column.id === "select" &&
                          row.getIsSelected() &&
                          "bg-muted/10",
                      )}
                      style={{
                        minWidth:
                          cell.column.id === "select" ? "40px" : "150px",
                        maxWidth:
                          cell.column.id === "select" ? "40px" : "300px",
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
