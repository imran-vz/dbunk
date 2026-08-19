/**
 * Toolbar and filter bar for the DataGrid. Extracted to keep the
 * parent component under the 1000-line threshold.
 */

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
import type { Table } from "@tanstack/react-table";
import { useCallback, useEffect, useState } from "react";

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
import { downloadBlob } from "@/lib/download";
import {
  type ExportCompression,
  type ExportEncoding,
  type ExportFormat,
  type ExportTable,
  prepareExportBlob,
} from "@/lib/export";
import { TABLE_BROWSE_PAGE_SIZES } from "@/lib/table-browse";
import { cn } from "@/lib/utils";

import {
  BrowseFilterBar,
  BrowseInspectionPanel,
  BrowseLiveRegion,
  BrowseSortEditor,
} from "./browse-controls";
import type { ServerBrowseGridModel } from "./browse-model";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AppliedFilter = {
  column: string;
  operator: string;
  value: string;
};

export type ExportSettings = {
  format: ExportFormat;
  encoding: ExportEncoding;
  compression: ExportCompression;
  nullAs: string;
};

export interface DataGridToolbarProps {
  table: Table<string[]>;
  columnNames: string[];
  hasEdits?: boolean;
  isSaving?: boolean;
  hasSelection: boolean;
  exportFilenameBase: string;
  toolbarLeading?: React.ReactNode;
  onSave?: () => void;
  onDiscard?: () => void;
  onRefresh?: () => void;
  onOpenSQL?: () => void;
  onExportWholeTable?: (settings: ExportSettings) => void;
  onSaveExportTask?: (settings: ExportSettings) => void;
  onRunSavedExportTask?: () => void;
  hasSavedExportTask?: boolean;
  buildExportTable: (mode: "all" | "selected") => ExportTable;
  appliedFilters: AppliedFilter[];
  onApplyFilter: (filter: AppliedFilter) => void;
  onRemoveFilter: (column: string) => void;
  onClearFilters: () => void;
  serverBrowse?: ServerBrowseGridModel;
  onExpandGrid?: () => void;
  expanded?: boolean;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function DataGridToolbar({
  table,
  columnNames,
  hasEdits,
  isSaving,
  hasSelection,
  exportFilenameBase,
  toolbarLeading,
  onSave,
  onDiscard,
  onRefresh,
  onOpenSQL,
  onExportWholeTable,
  onSaveExportTask,
  onRunSavedExportTask,
  hasSavedExportTask,
  buildExportTable,
  appliedFilters,
  onApplyFilter,
  onRemoveFilter,
  onClearFilters,
  serverBrowse,
  onExpandGrid,
  expanded,
}: DataGridToolbarProps) {
  const [showFilters, setShowFilters] = useState(false);
  const [showSort, setShowSort] = useState(false);
  const [showInspection, setShowInspection] = useState(false);
  const [columnSearch, setColumnSearch] = useState("");
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
    setDraftColumn((prev) =>
      columnNames.includes(prev) ? prev : (columnNames[0] ?? ""),
    );
  }, [columnNames]);

  const canApplyDraft = draftColumn !== "" && draftValue.trim() !== "";

  const applyDraft = useCallback(() => {
    if (!canApplyDraft) return;
    onApplyFilter({
      column: draftColumn,
      operator: draftOperator,
      value: draftValue.trim(),
    });
    setDraftValue("");
  }, [canApplyDraft, draftColumn, draftOperator, draftValue, onApplyFilter]);

  const handleExport = useCallback(
    async (mode: "all" | "selected", format: ExportFormat) => {
      const exportTable = buildExportTable(mode);
      const filenameBase = `${exportFilenameBase || "export"}${
        mode === "selected" ? "-selected" : ""
      }`;
      const { filename, blob } = await prepareExportBlob(exportTable, {
        format,
        filenameBase,
        encoding: exportEncoding,
        compression: exportCompression,
        nullAs: exportNullAs,
      });
      downloadBlob(filename, blob);
    },
    [
      buildExportTable,
      exportCompression,
      exportEncoding,
      exportFilenameBase,
      exportNullAs,
    ],
  );

  const exportSettings: ExportSettings = {
    format: exportFormat,
    encoding: exportEncoding,
    compression: exportCompression,
    nullAs: exportNullAs,
  };

  return (
    <>
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
            onClick={() => setShowSort((open) => !open)}
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
                        // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
                        typeof column.accessorFn !== "undefined" &&
                        column.getCanHide() &&
                        column.id
                          .toLowerCase()
                          .includes(columnSearch.toLowerCase()),
                    )
                    .map((column) => (
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
                    ))}
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

          {serverBrowse?.loadStatus.state === "loading" ? (
            <Button
              variant="outline"
              size="sm"
              className="gap-1.5 border-border-subtle bg-surface-panel"
              onClick={serverBrowse.onCancel}
              aria-label="Cancel browse"
              title="Cancel browse"
            >
              <IconX className="size-3.5" />
              <span className="dbunk-optional-label">Cancel</span>
            </Button>
          ) : null}

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
                  {serverBrowse ? "Current page" : "Current result rows"}
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
                      // SAFETY: The value is constrained by the typed component or library contract at this boundary.
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

          {serverBrowse ? (
            <Button
              variant={showInspection ? "secondary" : "outline"}
              size="sm"
              className="gap-1.5 border-border-subtle bg-surface-panel"
              aria-label="Inspect query"
              title="Inspect query"
              onClick={() => setShowInspection((open) => !open)}
            >
              SQL
            </Button>
          ) : null}

          {toolbarLeading ? (
            <>
              <div className="mx-0.5 h-5 w-px bg-border-subtle" />
              {toolbarLeading}
            </>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          {serverBrowse ? (
            <Select
              value={String(serverBrowse.pageSize)}
              onValueChange={(value) => {
                const next = Number.parseInt(value ?? "", 10);
                if (!Number.isFinite(next)) return;
                serverBrowse.onPageSizeChange(next);
              }}
            >
              <SelectTrigger
                className="h-6 w-24 border-border-subtle bg-surface-panel text-xs"
                aria-label="Page size"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {TABLE_BROWSE_PAGE_SIZES.map((size) => (
                  <SelectItem key={size} value={String(size)}>
                    {size} rows
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : (
            <Select defaultValue={String(table.getState().pagination.pageSize)}>
              <SelectTrigger
                className="h-6 w-24 border-border-subtle bg-surface-panel text-xs"
                aria-label="Page size"
              >
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
          )}
          <Button
            variant="ghost"
            size="icon"
            aria-label={expanded ? "Restore grid" : "Expand grid"}
            className="rounded-sm border border-border-subtle bg-surface-panel"
            onClick={onExpandGrid}
            disabled={!onExpandGrid}
          >
            <IconArrowsMaximize className="size-3.5" />
          </Button>
        </div>
      </div>

      {serverBrowse ? <BrowseLiveRegion browse={serverBrowse} /> : null}

      {showFilters && serverBrowse ? (
        <BrowseFilterBar columnNames={columnNames} browse={serverBrowse} />
      ) : null}

      {showSort && serverBrowse ? (
        <BrowseSortEditor columnNames={columnNames} browse={serverBrowse} />
      ) : null}

      {showInspection && serverBrowse ? (
        <div className="border-b border-border-subtle bg-surface-window px-3 py-2">
          <BrowseInspectionPanel browse={serverBrowse} />
        </div>
      ) : null}

      {showFilters && !serverBrowse && (
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
                    onClick={() => onRemoveFilter(f.column)}
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
              onClick={onClearFilters}
              disabled={appliedFilters.length === 0}
            >
              Clear filters
            </Button>
          </div>
        </div>
      )}
    </>
  );
}
