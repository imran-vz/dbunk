/**
 * Data grid (DESIGN-SYSTEM §5.4) — the one grid behind query results
 * and the table browser. Rebuilt in the P5 UI refresh:
 *
 * - Row + column virtualization with fixed `--row-grid` heights (only
 *   visible cells reach the DOM; 100k rows scroll flat).
 * - Content-derived initial column widths, drag-resize, double-click
 *   auto-fit, hide/pin-left, per-table width persistence
 *   (`gridLayoutKey`), and a header context menu.
 * - Focused-cell keyboard model: arrows/Shift-ranges, page/home/end,
 *   `Cmd+G` go-to-row, `Cmd+C` TSV + copy-as formats, `Space` value
 *   inspector, Enter/F2/double-click editing with staged commits.
 *
 * The measurement fallback matters: when ResizeObserver or a real
 * layout is unavailable (jsdom, first paint), the grid renders against
 * a default viewport instead of rendering nothing.
 */

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- virtualized ARIA grid: rows/cells are absolutely positioned divs; table tags cannot be virtualized. */

import type {
  Row,
  RowSelectionState,
  VisibilityState,
} from "@tanstack/react-table";
import {
  type ColumnDef,
  type ColumnFiltersState,
  getCoreRowModel,
  getFilteredRowModel,
  useReactTable,
} from "@tanstack/react-table";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import type { ServerBrowseGridModel } from "@/components/data-grid/browse-model";
import {
  type ColumnHeaderMeta,
  ColumnHeaderLabel,
  type ForeignKeyTarget,
  GridBodyCell,
} from "@/components/data-grid/grid-cells";
import {
  autoFitWidth,
  buildCopyText,
  COPY_FORMATS,
  type CopyFormat,
  columnOffsets,
  detectAlignment,
  estimateInitialWidths,
  loadGridLayout,
  MAX_AUTO_FIT_WIDTH,
  MIN_COLUMN_WIDTH,
  saveGridLayout,
  virtualColumnRange,
  virtualRowRange,
} from "@/components/data-grid/grid-model";
import {
  type AppliedFilter,
  DataGridToolbar,
  type ExportSettings,
} from "@/components/data-grid/toolbar";
import {
  type InspectedValue,
  ValueInspector,
} from "@/components/data-grid/value-inspector";
import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/state-panel";
import { useDensity } from "@/lib/density";
import type { ExportTable } from "@/lib/export";
import { shortcutKeys } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";

export type { ColumnHeaderMeta, ForeignKeyTarget };

export type TableViewMode = "data" | "structure";

export type DataGridRowState =
  | "deleted"
  | "inserted"
  | "duplicate"
  | "excluded";

export interface DataGridProps {
  data: string[][];
  columns: string[];
  /** Optional per-column Postgres data types, aligned to `columns`. */
  columnTypes?: Array<string | undefined>;
  /** Per-column structural metadata, aligned to `columns`. */
  columnMetadata?: Array<ColumnHeaderMeta | undefined>;
  onFollowForeignKey?: (
    rowIndex: number,
    target: ForeignKeyTarget,
    value: string,
  ) => void;
  edits?: Record<number, Record<number, string>>;
  onEdit?: (rowIndex: number, colIndex: number, value: string) => void;
  /** Invoked when a read-only cell is activated so its owner can lazily
   * analyze the result set before exposing an editor. */
  onEditIntent?: (rowIndex: number, colIndex: number) => void;
  /** A reason here disables editing for this cell while keeping the rest of
   * the row editable. The reason is exposed as the cell tooltip. */
  getCellReadOnlyReason?: (
    rowIndex: number,
    colIndex: number,
  ) => string | undefined;
  /** Presentation-only staged row state. Draft identity remains owned by the
   * caller; the grid never treats row indexes as mutation identity. */
  getRowState?: (rowIndex: number) => DataGridRowState | undefined;
  className?: string;
  onSave?: () => void;
  onDiscard?: () => void;
  onOpenSQL?: () => void;
  onRefresh?: () => void;
  hasEdits?: boolean;
  /** Suppresses cell editing entirely. */
  readOnly?: boolean;
  isSaving?: boolean;
  viewMode?: TableViewMode;
  onViewModeChange?: (mode: TableViewMode) => void;
  onToggleSidebar?: () => void;
  exportFilenameBase?: string;
  toolbarLeading?: React.ReactNode;
  rowSelection?: RowSelectionState;
  onRowSelectionChange?: (selection: RowSelectionState) => void;
  onExportWholeTable?: (options: ExportSettings) => Promise<void>;
  onSaveExportTask?: (options: ExportSettings) => void;
  onRunSavedExportTask?: () => Promise<void>;
  hasSavedExportTask?: boolean;
  /** Inline row expansion (FK drill-down) under the given data row. */
  rowExpansion?: {
    rowIndex: number;
    content: React.ReactNode;
  } | null;
  serverBrowse?: ServerBrowseGridModel;
  onExpandGrid?: () => void;
  expanded?: boolean;
  /**
   * Identity for persisted grid layout (column widths, pinned columns)
   * — pass `${connectionId}.${schema}.${table}` so widths persist per
   * table per connection (§5.4). Omit for ad-hoc results.
   */
  gridLayoutKey?: string;
  /** `Cmd+D` / context menu: clone the single checkbox-selected row (staged). */
  onCloneSelectedRow?: () => void;
  /** `Delete` / context menu: stage deletion of checkbox-selected rows. */
  onDeleteSelectedRows?: () => void;
}

const GUTTER_WIDTH = 40;
/** Fixed slot for the FK drill-down expansion; content scrolls inside. */
const EXPANSION_HEIGHT = 280;
const ROW_HEIGHTS = {
  compact: 22,
  default: 26,
  comfortable: 30,
} as const;

type CellPos = { row: number; col: number };

type GridSelection =
  | { kind: "cells"; anchor: CellPos; focus: CellPos }
  | { kind: "columns"; cols: number[] }
  | { kind: "rows"; rows: number[] }
  | { kind: "all" }
  | null;

type ContextTarget = { kind: "cell" } | { kind: "header"; col: number };

function isCellSelected(sel: GridSelection, row: number, col: number): boolean {
  if (!sel) return false;
  switch (sel.kind) {
    case "all":
      return true;
    case "columns":
      return sel.cols.includes(col);
    case "rows":
      return sel.rows.includes(row);
    case "cells": {
      const rowLow = Math.min(sel.anchor.row, sel.focus.row);
      const rowHigh = Math.max(sel.anchor.row, sel.focus.row);
      const colLow = Math.min(sel.anchor.col, sel.focus.col);
      const colHigh = Math.max(sel.anchor.col, sel.focus.col);
      return row >= rowLow && row <= rowHigh && col >= colLow && col <= colHigh;
    }
  }
}

const isPrintableKey = (event: React.KeyboardEvent): boolean =>
  event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey;

export function DataGrid({
  data,
  columns: columnNames,
  columnTypes,
  columnMetadata,
  onFollowForeignKey,
  rowExpansion,
  edits,
  onEdit,
  onEditIntent,
  getCellReadOnlyReason,
  getRowState,
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
  gridLayoutKey,
  onCloneSelectedRow,
  onDeleteSelectedRows,
}: DataGridProps) {
  const effectiveOnEdit = readOnly ? undefined : onEdit;
  const density = useDensity();
  const rowHeight = ROW_HEIGHTS[density];
  const headerHeight = rowHeight + 2;

  // ------------------------------------------------------------------
  // Table model (filtering, visibility, checkbox row selection)
  // ------------------------------------------------------------------
  const [appliedFilters, setAppliedFilters] = useState<AppliedFilter[]>([]);
  const [columnVisibility, setColumnVisibility] = useState<VisibilityState>({});
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
        // oxlint-disable-next-line anti-slop/no-runtime-typeof -- tanstack updater union.
        typeof updater === "function" ? updater(rowSelection) : updater;
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

  const columnDefs = useMemo<ColumnDef<string[]>[]>(
    () =>
      columnNames.map((name, index) => ({
        id: name,
        accessorFn: (row: string[]) => row[index],
      })),
    [columnNames],
  );

  const table = useReactTable({
    data,
    columns: columnDefs,
    getCoreRowModel: getCoreRowModel(),
    getFilteredRowModel: serverBrowse ? undefined : getFilteredRowModel(),
    onColumnVisibilityChange: setColumnVisibility,
    onRowSelectionChange: setRowSelection,
    state: {
      columnFilters: serverBrowse ? [] : columnFilters,
      columnVisibility,
      rowSelection,
    },
    manualFiltering: Boolean(serverBrowse),
  });

  const rows = table.getRowModel().rows;

  // ------------------------------------------------------------------
  // Column layout: order (pinned first), widths, alignment
  // ------------------------------------------------------------------
  const [layout, setLayout] = useState(() => loadGridLayout(gridLayoutKey));
  useEffect(() => {
    setLayout(loadGridLayout(gridLayoutKey));
  }, [gridLayoutKey]);
  useEffect(() => {
    saveGridLayout(gridLayoutKey, layout);
  }, [gridLayoutKey, layout]);

  const dataIndexByName = useMemo(() => {
    const map = new Map<string, number>();
    columnNames.forEach((name, index) => map.set(name, index));
    return map;
  }, [columnNames]);

  const visibleCols = useMemo(() => {
    const visibleNames = columnNames.filter(
      (name) => columnVisibility[name] !== false,
    );
    const pinned = layout.pinned.filter((name) => visibleNames.includes(name));
    const unpinned = visibleNames.filter((name) => !pinned.includes(name));
    return [...pinned, ...unpinned].map((name) => ({
      name,
      dataIndex: dataIndexByName.get(name) ?? 0,
    }));
  }, [columnNames, columnVisibility, layout.pinned, dataIndexByName]);
  const pinnedCount = useMemo(
    () =>
      layout.pinned.filter((name) =>
        visibleCols.some((col) => col.name === name),
      ).length,
    [layout.pinned, visibleCols],
  );

  const initialWidths = useMemo(
    () => estimateInitialWidths(columnNames, data),
    [columnNames, data],
  );
  const colWidths = useMemo(
    () =>
      visibleCols.map(
        (col) => layout.widths[col.name] ?? initialWidths[col.dataIndex] ?? 150,
      ),
    [visibleCols, layout.widths, initialWidths],
  );
  const offsets = useMemo(() => columnOffsets(colWidths), [colWidths]);
  const totalColumnWidth = offsets[offsets.length - 1] ?? 0;
  const pinnedWidth = offsets[pinnedCount] ?? 0;

  const alignments = useMemo(
    () =>
      visibleCols.map((col) =>
        detectAlignment(columnTypes?.[col.dataIndex], col.dataIndex, data),
      ),
    [visibleCols, columnTypes, data],
  );

  // ------------------------------------------------------------------
  // Viewport + virtualization (with a no-measurement fallback)
  // ------------------------------------------------------------------
  const scrollRef = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ width: 800, height: 600 });
  const [scroll, setScroll] = useState({ top: 0, left: 0 });

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () =>
      setViewport({
        width: el.clientWidth || 800,
        height: el.clientHeight || 600,
      });
    measure();
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- ResizeObserver is an optional runtime capability (absent in jsdom).
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  const expansionDisplayIndex = useMemo(() => {
    if (!rowExpansion || rowExpansion.content === null) return -1;
    return rows.findIndex((row) => row.index === rowExpansion.rowIndex);
  }, [rowExpansion, rows]);
  const expansionActive = expansionDisplayIndex !== -1;

  const bodyHeight = Math.max(0, viewport.height - headerHeight);
  const rowRange = virtualRowRange(
    scroll.top,
    bodyHeight,
    rows.length,
    rowHeight,
  );
  const colRange = virtualColumnRange(scroll.left, viewport.width, offsets);

  const rowTop = useCallback(
    (displayIndex: number) =>
      headerHeight +
      displayIndex * rowHeight +
      (expansionActive && displayIndex > expansionDisplayIndex
        ? EXPANSION_HEIGHT
        : 0),
    [headerHeight, rowHeight, expansionActive, expansionDisplayIndex],
  );
  const totalHeight =
    headerHeight +
    rows.length * rowHeight +
    (expansionActive ? EXPANSION_HEIGHT : 0);
  const totalWidth = GUTTER_WIDTH + totalColumnWidth;

  // ------------------------------------------------------------------
  // Focus, selection, editing
  // ------------------------------------------------------------------
  const [focused, setFocused] = useState<CellPos | null>(null);
  const [selection, setSelection] = useState<GridSelection>(null);
  const [expandStage, setExpandStage] = useState(0);
  const [editing, setEditing] = useState<{
    pos: CellPos;
    seed?: string;
  } | null>(null);
  const [inspected, setInspected] = useState<InspectedValue | null>(null);
  const [goToOpen, setGoToOpen] = useState(false);
  const [contextTarget, setContextTarget] = useState<ContextTarget>({
    kind: "cell",
  });
  const wantFocusRef = useRef(false);

  const cellValue = useCallback(
    (displayRow: number, visibleCol: number): string => {
      const row = rows[displayRow];
      const col = visibleCols[visibleCol];
      if (!row || !col) return "";
      return (
        edits?.[row.index]?.[col.dataIndex] ?? row.original[col.dataIndex] ?? ""
      );
    },
    [rows, visibleCols, edits],
  );

  const clampPos = useCallback(
    (pos: CellPos): CellPos => ({
      row: Math.max(0, Math.min(rows.length - 1, pos.row)),
      col: Math.max(0, Math.min(visibleCols.length - 1, pos.col)),
    }),
    [rows.length, visibleCols.length],
  );

  const ensureVisible = useCallback(
    (pos: CellPos) => {
      const el = scrollRef.current;
      if (!el) return;
      const top = rowTop(pos.row) - headerHeight;
      const bottom = top + rowHeight;
      if (top < el.scrollTop) el.scrollTop = top;
      else if (bottom > el.scrollTop + bodyHeight) {
        el.scrollTop = bottom - bodyHeight;
      }
      if (pos.col >= pinnedCount) {
        const left = GUTTER_WIDTH + (offsets[pos.col] ?? 0);
        const right = left + (colWidths[pos.col] ?? 0);
        const frozen = GUTTER_WIDTH + pinnedWidth;
        if (left - frozen < el.scrollLeft) {
          el.scrollLeft = Math.max(0, left - frozen);
        } else if (right > el.scrollLeft + viewport.width) {
          el.scrollLeft = right - viewport.width;
        }
      }
    },
    [
      rowTop,
      headerHeight,
      rowHeight,
      bodyHeight,
      offsets,
      colWidths,
      pinnedCount,
      pinnedWidth,
      viewport.width,
    ],
  );

  // Focus follows the focused-cell state after keyboard moves.
  useEffect(() => {
    if (!wantFocusRef.current || !focused || editing) return;
    wantFocusRef.current = false;
    const el = scrollRef.current;
    if (!el) return;
    const cell = el.querySelector<HTMLElement>(
      `[data-grid-cell="${focused.row}-${focused.col}"]`,
    );
    (cell ?? el).focus({ preventScroll: true });
  });

  const activateCell = useCallback(
    (pos: CellPos, extend: boolean) => {
      wantFocusRef.current = true;
      setExpandStage(0);
      setFocused(pos);
      setSelection((current) =>
        extend && current?.kind === "cells"
          ? { kind: "cells", anchor: current.anchor, focus: pos }
          : { kind: "cells", anchor: pos, focus: pos },
      );
      const row = rows[pos.row];
      const col = visibleCols[pos.col];
      if (!effectiveOnEdit && onEditIntent && row && col) {
        onEditIntent(row.index, col.dataIndex);
      }
    },
    [rows, visibleCols, effectiveOnEdit, onEditIntent],
  );

  const moveFocus = useCallback(
    (deltaRow: number, deltaCol: number, extend: boolean) => {
      if (rows.length === 0 || visibleCols.length === 0) return;
      const from = focused ?? { row: 0, col: 0 };
      const next = clampPos({
        row: from.row + deltaRow,
        col: from.col + deltaCol,
      });
      wantFocusRef.current = true;
      setExpandStage(0);
      setFocused(next);
      setSelection((current) =>
        extend
          ? {
              kind: "cells",
              anchor: current?.kind === "cells" ? current.anchor : from,
              focus: next,
            }
          : { kind: "cells", anchor: next, focus: next },
      );
      ensureVisible(next);
    },
    [rows.length, visibleCols.length, focused, clampPos, ensureVisible],
  );

  const jumpFocus = useCallback(
    (pos: CellPos) => {
      if (rows.length === 0 || visibleCols.length === 0) return;
      const next = clampPos(pos);
      wantFocusRef.current = true;
      setFocused(next);
      setSelection({ kind: "cells", anchor: next, focus: next });
      ensureVisible(next);
    },
    [rows.length, visibleCols.length, clampPos, ensureVisible],
  );

  const startEdit = useCallback(
    (pos: CellPos, seed?: string) => {
      const row = rows[pos.row];
      const col = visibleCols[pos.col];
      if (!row || !col) return;
      if (!effectiveOnEdit) {
        onEditIntent?.(row.index, col.dataIndex);
        return;
      }
      if (getCellReadOnlyReason?.(row.index, col.dataIndex)) return;
      setFocused(pos);
      setEditing({ pos, seed });
    },
    [rows, visibleCols, effectiveOnEdit, onEditIntent, getCellReadOnlyReason],
  );

  const commitEdit = useCallback(
    (value: string, moveRight: boolean) => {
      const current = editing;
      setEditing(null);
      wantFocusRef.current = true;
      if (!current) return;
      const row = rows[current.pos.row];
      const col = visibleCols[current.pos.col];
      if (row && col && effectiveOnEdit) {
        const original = row.original[col.dataIndex] ?? "";
        const isDirty = edits?.[row.index]?.[col.dataIndex] !== undefined;
        if (value !== original || isDirty) {
          effectiveOnEdit(row.index, col.dataIndex, value);
        }
      }
      if (moveRight) moveFocus(0, 1, false);
    },
    [editing, rows, visibleCols, effectiveOnEdit, edits, moveFocus],
  );

  const openInspector = useCallback(
    (pos: CellPos) => {
      const col = visibleCols[pos.col];
      if (!col) return;
      setInspected({ column: col.name, value: cellValue(pos.row, pos.col) });
    },
    [visibleCols, cellValue],
  );

  // ------------------------------------------------------------------
  // Copy
  // ------------------------------------------------------------------
  const buildSelectionTable = useCallback((): ExportTable | null => {
    if (rows.length === 0 || visibleCols.length === 0) return null;
    const sel: GridSelection =
      selection ??
      (focused ? { kind: "cells", anchor: focused, focus: focused } : null);
    if (!sel) return null;
    let rowIndexes: number[] = [];
    let colIndexes: number[] = [];
    const allRows = rows.map((_row, index) => index);
    const allCols = visibleCols.map((_col, index) => index);
    switch (sel.kind) {
      case "all":
        rowIndexes = allRows;
        colIndexes = allCols;
        break;
      case "columns":
        rowIndexes = allRows;
        colIndexes = sel.cols;
        break;
      case "rows":
        rowIndexes = sel.rows;
        colIndexes = allCols;
        break;
      case "cells": {
        const rowLow = Math.min(sel.anchor.row, sel.focus.row);
        const rowHigh = Math.max(sel.anchor.row, sel.focus.row);
        const colLow = Math.min(sel.anchor.col, sel.focus.col);
        const colHigh = Math.max(sel.anchor.col, sel.focus.col);
        for (let r = rowLow; r <= rowHigh; r += 1) rowIndexes.push(r);
        for (let c = colLow; c <= colHigh; c += 1) colIndexes.push(c);
        break;
      }
    }
    return {
      columns: colIndexes.map((c) => visibleCols[c]?.name ?? ""),
      rows: rowIndexes.map((r) =>
        colIndexes.map((c) => {
          const value = cellValue(r, c);
          return value === "NULL" ? null : value;
        }),
      ),
    };
  }, [rows, visibleCols, selection, focused, cellValue]);

  const copySelection = useCallback(
    async (format: CopyFormat) => {
      const selectionTable = buildSelectionTable();
      if (!selectionTable) return;
      const text = buildCopyText(format, selectionTable, exportFilenameBase);
      try {
        await navigator.clipboard.writeText(text);
        const cellCount =
          selectionTable.rows.length * selectionTable.columns.length;
        toast.success(
          `Copied ${cellCount} ${cellCount === 1 ? "cell" : "cells"}`,
        );
      } catch {
        toast.error("Copy failed.");
      }
    },
    [buildSelectionTable, exportFilenameBase],
  );

  // ------------------------------------------------------------------
  // Column layout actions
  // ------------------------------------------------------------------
  const setColumnWidth = useCallback((name: string, width: number) => {
    setLayout((current) => ({
      ...current,
      widths: {
        ...current.widths,
        [name]: Math.max(
          MIN_COLUMN_WIDTH,
          Math.min(MAX_AUTO_FIT_WIDTH, Math.round(width)),
        ),
      },
    }));
  }, []);

  const autoFitColumn = useCallback(
    (visibleIndex: number) => {
      const col = visibleCols[visibleIndex];
      if (!col) return;
      setColumnWidth(col.name, autoFitWidth(col.name, col.dataIndex, data));
    },
    [visibleCols, data, setColumnWidth],
  );

  const autoFitAllColumns = useCallback(() => {
    setLayout((current) => {
      const widths = { ...current.widths };
      for (const col of visibleCols) {
        widths[col.name] = autoFitWidth(col.name, col.dataIndex, data);
      }
      return { ...current, widths };
    });
  }, [visibleCols, data]);

  const togglePinned = useCallback((name: string) => {
    setLayout((current) => ({
      ...current,
      pinned: current.pinned.includes(name)
        ? current.pinned.filter((pin) => pin !== name)
        : [...current.pinned, name],
    }));
  }, []);

  const startResize = useCallback(
    (event: React.PointerEvent, visibleIndex: number) => {
      event.preventDefault();
      event.stopPropagation();
      const col = visibleCols[visibleIndex];
      if (!col) return;
      const startX = event.clientX;
      const startWidth = colWidths[visibleIndex] ?? 150;
      const onMove = (move: PointerEvent) => {
        setColumnWidth(col.name, startWidth + (move.clientX - startX));
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    },
    [visibleCols, colWidths, setColumnWidth],
  );

  // ------------------------------------------------------------------
  // Keyboard
  // ------------------------------------------------------------------
  const pageRows = Math.max(1, Math.floor(bodyHeight / rowHeight) - 1);
  const selectedRowCount = Object.keys(rowSelection).length;

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        target.closest("input, textarea, [data-grid-editing]")
      ) {
        return;
      }
      const mod = event.metaKey || event.ctrlKey;
      const pos = focused ?? { row: 0, col: 0 };
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          if (event.altKey) return;
          moveFocus(1, 0, event.shiftKey);
          return;
        case "ArrowUp":
          if (event.altKey) {
            // Expanding selection (§5.4): cell → column → row → grid.
            event.preventDefault();
            if (!focused) return;
            const stage = expandStage + 1;
            setExpandStage(stage);
            if (stage === 1) {
              setSelection({ kind: "columns", cols: [focused.col] });
            } else if (stage === 2) {
              setSelection({ kind: "rows", rows: [focused.row] });
            } else {
              setSelection({ kind: "all" });
            }
            return;
          }
          event.preventDefault();
          moveFocus(-1, 0, event.shiftKey);
          return;
        case "ArrowLeft":
          event.preventDefault();
          moveFocus(0, -1, event.shiftKey);
          return;
        case "ArrowRight":
          event.preventDefault();
          moveFocus(0, 1, event.shiftKey);
          return;
        case "PageDown":
          event.preventDefault();
          moveFocus(pageRows, 0, event.shiftKey);
          return;
        case "PageUp":
          event.preventDefault();
          moveFocus(-pageRows, 0, event.shiftKey);
          return;
        case "Home":
          event.preventDefault();
          jumpFocus(mod ? { row: 0, col: 0 } : { row: pos.row, col: 0 });
          return;
        case "End":
          event.preventDefault();
          jumpFocus(
            mod
              ? { row: rows.length - 1, col: visibleCols.length - 1 }
              : { row: pos.row, col: visibleCols.length - 1 },
          );
          return;
        case "Enter":
          event.preventDefault();
          if (event.shiftKey) {
            if (focused) openInspector(focused);
            return;
          }
          startEdit(pos);
          return;
        case "F2":
          event.preventDefault();
          startEdit(pos);
          return;
        case " ":
          event.preventDefault();
          if (focused) openInspector(focused);
          return;
        case "Escape":
          setSelection(null);
          return;
        case "Delete":
        case "Backspace":
          if (selectedRowCount > 0 && onDeleteSelectedRows) {
            event.preventDefault();
            onDeleteSelectedRows();
          }
          return;
        default:
          break;
      }
      if (mod) {
        const key = event.key.toLowerCase();
        if (key === "c") {
          event.preventDefault();
          void copySelection("tsv");
        } else if (key === "a") {
          event.preventDefault();
          setSelection({ kind: "all" });
        } else if (key === "g") {
          event.preventDefault();
          setGoToOpen(true);
        } else if (key === "d") {
          if (selectedRowCount === 1 && onCloneSelectedRow) {
            event.preventDefault();
            onCloneSelectedRow();
          }
        }
        return;
      }
      if (isPrintableKey(event) && focused && effectiveOnEdit) {
        event.preventDefault();
        startEdit(focused, event.key);
      }
    },
    [
      focused,
      expandStage,
      moveFocus,
      jumpFocus,
      pageRows,
      rows.length,
      visibleCols.length,
      startEdit,
      openInspector,
      copySelection,
      selectedRowCount,
      onDeleteSelectedRows,
      onCloneSelectedRow,
      effectiveOnEdit,
    ],
  );

  // ------------------------------------------------------------------
  // Export (toolbar contract)
  // ------------------------------------------------------------------
  const buildExportTable = useCallback(
    (mode: "all" | "selected"): ExportTable => {
      const rowsModel =
        mode === "selected"
          ? table.getSelectedRowModel().rows
          : serverBrowse
            ? table.getRowModel().rows
            : table.getFilteredRowModel().rows;
      const exportRows = rowsModel.map((row) =>
        columnNames.map((_col, colIndex) => row.original[colIndex] ?? null),
      );
      return { columns: columnNames, rows: exportRows };
    },
    [table, columnNames, serverBrowse],
  );

  const hasSelection = selectedRowCount > 0;
  const focusedFkTarget =
    focused !== null
      ? columnMetadata?.[visibleCols[focused.col]?.dataIndex ?? -1]
          ?.foreignKeyTarget
      : undefined;

  // ------------------------------------------------------------------
  // Rendering
  // ------------------------------------------------------------------
  const renderColIndexes = useMemo(() => {
    const indexes: number[] = [];
    for (let index = 0; index < pinnedCount; index += 1) indexes.push(index);
    for (
      let index = Math.max(pinnedCount, colRange.start);
      index < colRange.end;
      index += 1
    ) {
      indexes.push(index);
    }
    return indexes;
  }, [pinnedCount, colRange.start, colRange.end]);

  const virtualRows = rows.slice(rowRange.start, rowRange.end);

  const headerCells = renderColIndexes.map((visibleIndex) => {
    const col = visibleCols[visibleIndex];
    if (!col) return null;
    const meta = columnMetadata?.[col.dataIndex];
    const isPinned = visibleIndex < pinnedCount;
    return (
      <div
        key={col.name}
        role="columnheader"
        aria-colindex={visibleIndex + 2}
        tabIndex={-1}
        data-grid-header={visibleIndex}
        onContextMenu={() =>
          setContextTarget({ kind: "header", col: visibleIndex })
        }
        className={cn(
          "absolute top-0 flex h-full items-center border-r border-b border-border-subtle bg-surface-sidebar",
          isPinned && "sticky z-30",
        )}
        style={
          isPinned
            ? {
                position: "sticky",
                left: GUTTER_WIDTH + (offsets[visibleIndex] ?? 0),
                width: colWidths[visibleIndex],
                flex: "none",
              }
            : {
                left: GUTTER_WIDTH + (offsets[visibleIndex] ?? 0),
                width: colWidths[visibleIndex],
              }
        }
      >
        <ColumnHeaderLabel
          name={col.name}
          meta={meta}
          onSortClick={
            serverBrowse
              ? (event) => serverBrowse.onHeaderSort(col.name, event.shiftKey)
              : undefined
          }
        />
        <button
          type="button"
          tabIndex={-1}
          aria-label={`Resize column ${col.name}`}
          className="absolute top-0 -right-[3px] z-10 h-full w-[7px] cursor-col-resize bg-transparent hover:bg-accent/40"
          onPointerDown={(event) => startResize(event, visibleIndex)}
          onDoubleClick={(event) => {
            event.preventDefault();
            autoFitColumn(visibleIndex);
          }}
        />
      </div>
    );
  });

  return (
    <div
      data-slot="data-grid"
      className={cn("flex h-full min-h-0 flex-col bg-surface-app", className)}
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
        onApplyFilter={(filter) =>
          setAppliedFilters((prev) => {
            const index = prev.findIndex((f) => f.column === filter.column);
            if (index >= 0) {
              const copy = prev.slice();
              copy[index] = filter;
              return copy;
            }
            return [...prev, filter];
          })
        }
        onRemoveFilter={(column) =>
          setAppliedFilters((prev) => prev.filter((f) => f.column !== column))
        }
        onClearFilters={() => setAppliedFilters([])}
        serverBrowse={serverBrowse}
        onExpandGrid={onExpandGrid}
        expanded={expanded}
      />

      {rows.length === 0 ? (
        <EmptyState title="No data available" />
      ) : (
        <ContextMenu>
          <ContextMenuTrigger
            render={
              <div
                ref={scrollRef}
                data-slot="data-grid-scroll"
                role="grid"
                aria-rowcount={rows.length + 1}
                aria-colcount={visibleCols.length + 1}
                tabIndex={0}
                onKeyDown={handleKeyDown}
                onScroll={(event) =>
                  setScroll({
                    top: event.currentTarget.scrollTop,
                    left: event.currentTarget.scrollLeft,
                  })
                }
                className={cn(
                  "relative min-h-0 flex-1 overflow-auto bg-surface-app outline-none",
                  serverBrowse?.loadStatus.state === "loading" && "opacity-60",
                )}
              />
            }
          >
            <div
              style={{
                height: totalHeight,
                width: totalWidth,
                position: "relative",
              }}
            >
              {/* Header row */}
              {/* Flex container: pinned header cells use inline
                  position:sticky, which puts them back in normal flow — in
                  block flow they would stack vertically below the gutter. */}
              <div
                role="row"
                aria-rowindex={1}
                className="sticky top-0 z-20 flex"
                style={{ height: headerHeight, width: totalWidth }}
              >
                <div
                  className="sticky left-0 z-40 flex h-full shrink-0 items-center justify-center border-r border-b border-border-subtle bg-surface-sidebar"
                  style={{ width: GUTTER_WIDTH, position: "sticky" }}
                >
                  <input
                    type="checkbox"
                    aria-label="Select all rows"
                    className="size-3.5 rounded-sm border-border-strong bg-transparent accent-accent"
                    checked={table.getIsAllRowsSelected()}
                    onChange={table.getToggleAllRowsSelectedHandler()}
                  />
                </div>
                {headerCells}
              </div>

              {/* Virtual rows */}
              {virtualRows.map((row, sliceIndex) => {
                const displayIndex = rowRange.start + sliceIndex;
                const rowState = getRowState?.(row.index);
                return (
                  <GridRow
                    key={row.id}
                    row={row}
                    displayIndex={displayIndex}
                    top={rowTop(displayIndex)}
                    height={rowHeight}
                    width={totalWidth}
                    rowState={rowState}
                    renderColIndexes={renderColIndexes}
                    visibleCols={visibleCols}
                    pinnedCount={pinnedCount}
                    offsets={offsets}
                    colWidths={colWidths}
                    alignments={alignments}
                    edits={edits}
                    focused={focused}
                    selection={selection}
                    editing={editing}
                    effectiveOnEdit={effectiveOnEdit}
                    getCellReadOnlyReason={getCellReadOnlyReason}
                    columnTypes={columnTypes}
                    columnMetadata={columnMetadata}
                    onFollowForeignKey={onFollowForeignKey}
                    activateCell={activateCell}
                    startEdit={startEdit}
                    commitEdit={commitEdit}
                    cancelEdit={() => {
                      wantFocusRef.current = true;
                      setEditing(null);
                    }}
                  />
                );
              })}

              {/* FK drill-down expansion slot */}
              {expansionActive ? (
                <div
                  className="absolute right-0 left-0 z-10 overflow-auto border-b border-border-subtle bg-surface-panel/60"
                  style={{
                    top: rowTop(expansionDisplayIndex) + rowHeight,
                    height: EXPANSION_HEIGHT,
                    width: totalWidth,
                  }}
                >
                  <div
                    className="sticky left-0"
                    style={{ width: viewport.width }}
                  >
                    {rowExpansion?.content}
                  </div>
                </div>
              ) : null}
            </div>
          </ContextMenuTrigger>
          <ContextMenuContent className="w-56">
            {contextTarget.kind === "header" ? (
              <HeaderMenuItems
                columnName={visibleCols[contextTarget.col]?.name ?? ""}
                isPinned={contextTarget.col < pinnedCount}
                serverBrowse={serverBrowse}
                onHide={() => {
                  const name = visibleCols[contextTarget.col]?.name;
                  if (name) table.getColumn(name)?.toggleVisibility(false);
                }}
                onAutoFit={() => autoFitColumn(contextTarget.col)}
                onAutoFitAll={autoFitAllColumns}
                onTogglePin={() => {
                  const name = visibleCols[contextTarget.col]?.name;
                  if (name) togglePinned(name);
                }}
              />
            ) : (
              <>
                <ContextMenuItem onClick={() => void copySelection("tsv")}>
                  Copy
                  <ContextMenuShortcut keys={shortcutKeys("copy-selection")} />
                </ContextMenuItem>
                <ContextMenuSub>
                  <ContextMenuSubTrigger>Copy as</ContextMenuSubTrigger>
                  <ContextMenuSubContent>
                    {COPY_FORMATS.map((format) => (
                      <ContextMenuItem
                        key={format.id}
                        onClick={() => void copySelection(format.id)}
                      >
                        {format.label}
                      </ContextMenuItem>
                    ))}
                  </ContextMenuSubContent>
                </ContextMenuSub>
                <ContextMenuItem
                  disabled={!focused}
                  onClick={() => {
                    if (focused) openInspector(focused);
                  }}
                >
                  Inspect value
                  <ContextMenuShortcut keys={shortcutKeys("inspect-value")} />
                </ContextMenuItem>
                {focused && focusedFkTarget && onFollowForeignKey ? (
                  <ContextMenuItem
                    onClick={() => {
                      const row = rows[focused.row];
                      if (!row) return;
                      onFollowForeignKey(
                        row.index,
                        focusedFkTarget,
                        cellValue(focused.row, focused.col),
                      );
                    }}
                  >
                    Follow foreign key
                  </ContextMenuItem>
                ) : null}
                <ContextMenuSeparator />
                <ContextMenuItem
                  disabled={!focused}
                  onClick={() => {
                    if (focused) {
                      setSelection({ kind: "rows", rows: [focused.row] });
                    }
                  }}
                >
                  Select row
                </ContextMenuItem>
                <ContextMenuItem
                  disabled={!focused}
                  onClick={() => {
                    if (focused) {
                      setSelection({ kind: "columns", cols: [focused.col] });
                    }
                  }}
                >
                  Select column
                </ContextMenuItem>
                <ContextMenuItem onClick={() => setSelection({ kind: "all" })}>
                  Select all
                  <ContextMenuShortcut
                    keys={shortcutKeys("select-all-cells")}
                  />
                </ContextMenuItem>
                <ContextMenuItem onClick={() => setGoToOpen(true)}>
                  Go to row…
                  <ContextMenuShortcut keys={shortcutKeys("go-to-row")} />
                </ContextMenuItem>
                {onCloneSelectedRow || onDeleteSelectedRows ? (
                  <>
                    <ContextMenuSeparator />
                    {onCloneSelectedRow ? (
                      <ContextMenuItem
                        disabled={selectedRowCount !== 1}
                        onClick={onCloneSelectedRow}
                      >
                        Clone selected row
                        <ContextMenuShortcut keys={shortcutKeys("clone-row")} />
                      </ContextMenuItem>
                    ) : null}
                    {onDeleteSelectedRows ? (
                      <ContextMenuItem
                        variant="destructive"
                        disabled={selectedRowCount === 0}
                        onClick={onDeleteSelectedRows}
                      >
                        Delete selected rows
                        <ContextMenuShortcut
                          keys={shortcutKeys("delete-rows")}
                        />
                      </ContextMenuItem>
                    ) : null}
                  </>
                ) : null}
              </>
            )}
          </ContextMenuContent>
        </ContextMenu>
      )}

      <ValueInspector
        inspected={inspected}
        onClose={() => setInspected(null)}
      />
      <GoToRowDialog
        open={goToOpen}
        rowCount={rows.length}
        onClose={() => setGoToOpen(false)}
        onGo={(rowNumber) => {
          setGoToOpen(false);
          jumpFocus({ row: rowNumber - 1, col: focused?.col ?? 0 });
        }}
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Row
// ---------------------------------------------------------------------------

function GridRow({
  row,
  displayIndex,
  top,
  height,
  width,
  rowState,
  renderColIndexes,
  visibleCols,
  pinnedCount,
  offsets,
  colWidths,
  alignments,
  edits,
  focused,
  selection,
  editing,
  effectiveOnEdit,
  getCellReadOnlyReason,
  columnTypes,
  columnMetadata,
  onFollowForeignKey,
  activateCell,
  startEdit,
  commitEdit,
  cancelEdit,
}: {
  row: Row<string[]>;
  displayIndex: number;
  top: number;
  height: number;
  width: number;
  rowState: DataGridRowState | undefined;
  renderColIndexes: number[];
  visibleCols: Array<{ name: string; dataIndex: number }>;
  pinnedCount: number;
  offsets: number[];
  colWidths: number[];
  alignments: Array<"left" | "right" | "center">;
  edits?: Record<number, Record<number, string>>;
  focused: CellPos | null;
  selection: GridSelection;
  editing: { pos: CellPos; seed?: string } | null;
  effectiveOnEdit?: (rowIndex: number, colIndex: number, value: string) => void;
  getCellReadOnlyReason?: (
    rowIndex: number,
    colIndex: number,
  ) => string | undefined;
  columnTypes?: Array<string | undefined>;
  columnMetadata?: Array<ColumnHeaderMeta | undefined>;
  onFollowForeignKey?: (
    rowIndex: number,
    target: ForeignKeyTarget,
    value: string,
  ) => void;
  activateCell: (pos: CellPos, extend: boolean) => void;
  startEdit: (pos: CellPos, seed?: string) => void;
  commitEdit: (value: string, moveRight: boolean) => void;
  cancelEdit: () => void;
}) {
  return (
    <div
      role="row"
      aria-rowindex={displayIndex + 2}
      data-row-state={rowState}
      className={cn(
        "group/row absolute flex bg-surface-app",
        "hover:bg-surface-row-hover",
        row.getIsSelected() && "bg-accent-overlay",
        rowState === "deleted" && "bg-danger/10 text-text-muted",
        rowState === "inserted" && "bg-success/10",
        rowState === "duplicate" && "bg-warning/10",
        rowState === "excluded" && "opacity-50",
      )}
      style={{ top, height, width, position: "absolute" }}
    >
      {/* Row gutter: checkbox selection (§5.4 "click gutter = row"). */}
      <div
        className="sticky left-0 z-[2] flex h-full shrink-0 items-center justify-center border-r border-b border-border-subtle bg-inherit"
        style={{ width: GUTTER_WIDTH }}
      >
        <input
          type="checkbox"
          aria-label={`Select row ${displayIndex + 1}`}
          className="size-3.5 rounded-sm border-border-strong bg-transparent accent-accent"
          checked={row.getIsSelected()}
          onChange={row.getToggleSelectedHandler()}
        />
      </div>
      {renderColIndexes.map((visibleIndex) => {
        const col = visibleCols[visibleIndex];
        if (!col) return null;
        const isPinned = visibleIndex < pinnedCount;
        const editValue = edits?.[row.index]?.[col.dataIndex];
        const original = row.original[col.dataIndex] ?? "";
        const value = editValue ?? original;
        const pos = { row: displayIndex, col: visibleIndex };
        const isFocused =
          focused?.row === displayIndex && focused.col === visibleIndex;
        const isEditingCell =
          editing?.pos.row === displayIndex && editing.pos.col === visibleIndex;
        const readOnlyReason = getCellReadOnlyReason?.(
          row.index,
          col.dataIndex,
        );
        const fkTarget = columnMetadata?.[col.dataIndex]?.foreignKeyTarget;
        return (
          <GridBodyCell
            key={col.name}
            value={value}
            originalValue={original}
            isDirty={editValue !== undefined}
            alignment={alignments[visibleIndex] ?? "left"}
            isFocused={isFocused}
            isSelected={isCellSelected(selection, displayIndex, visibleIndex)}
            isEditing={isEditingCell}
            editSeed={isEditingCell ? editing.seed : undefined}
            editable={Boolean(effectiveOnEdit) && !readOnlyReason}
            lineThrough={rowState === "deleted"}
            readOnlyReason={readOnlyReason}
            columnName={col.name}
            columnType={columnTypes?.[col.dataIndex]}
            cellKey={`${displayIndex}-${visibleIndex}`}
            ariaColIndex={visibleIndex + 2}
            className={cn(isPinned && "z-[1] bg-inherit")}
            style={
              isPinned
                ? {
                    position: "sticky",
                    left: GUTTER_WIDTH + (offsets[visibleIndex] ?? 0),
                    width: colWidths[visibleIndex],
                    height: "100%",
                    flex: "none",
                  }
                : {
                    left: GUTTER_WIDTH + (offsets[visibleIndex] ?? 0),
                    width: colWidths[visibleIndex],
                    height: "100%",
                  }
            }
            onActivate={(extend) => activateCell(pos, extend)}
            onStartEdit={() => startEdit(pos)}
            onCommit={commitEdit}
            onCancelEdit={cancelEdit}
            foreignKeyTarget={fkTarget}
            onFollowForeignKey={
              fkTarget && onFollowForeignKey
                ? () => onFollowForeignKey(row.index, fkTarget, value)
                : undefined
            }
          />
        );
      })}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Header context menu items
// ---------------------------------------------------------------------------

function HeaderMenuItems({
  columnName,
  isPinned,
  serverBrowse,
  onHide,
  onAutoFit,
  onAutoFitAll,
  onTogglePin,
}: {
  columnName: string;
  isPinned: boolean;
  serverBrowse?: ServerBrowseGridModel;
  onHide: () => void;
  onAutoFit: () => void;
  onAutoFitAll: () => void;
  onTogglePin: () => void;
}) {
  return (
    <>
      {serverBrowse ? (
        <>
          <ContextMenuItem
            onClick={() =>
              serverBrowse.onSortChange([
                { column: columnName, direction: "asc", nulls: "default" },
              ])
            }
          >
            Sort ascending
          </ContextMenuItem>
          <ContextMenuItem
            onClick={() =>
              serverBrowse.onSortChange([
                { column: columnName, direction: "desc", nulls: "default" },
              ])
            }
          >
            Sort descending
          </ContextMenuItem>
          <ContextMenuItem onClick={() => serverBrowse.onSortChange([])}>
            Clear sort
          </ContextMenuItem>
          <ContextMenuSeparator />
        </>
      ) : null}
      <ContextMenuItem onClick={onAutoFit}>Auto-fit column</ContextMenuItem>
      <ContextMenuItem onClick={onAutoFitAll}>
        Auto-fit all columns
      </ContextMenuItem>
      <ContextMenuItem onClick={onTogglePin}>
        {isPinned ? "Unpin column" : "Pin column left"}
      </ContextMenuItem>
      <ContextMenuItem onClick={onHide}>Hide column</ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem
        onClick={() => void navigator.clipboard.writeText(columnName)}
      >
        Copy column name
      </ContextMenuItem>
    </>
  );
}

// ---------------------------------------------------------------------------
// Go-to-row dialog (Cmd+G)
// ---------------------------------------------------------------------------

function GoToRowDialog({
  open,
  rowCount,
  onClose,
  onGo,
}: {
  open: boolean;
  rowCount: number;
  onClose: () => void;
  onGo: (rowNumber: number) => void;
}) {
  const [value, setValue] = useState("");
  const submit = () => {
    const rowNumber = Number.parseInt(value, 10);
    if (Number.isFinite(rowNumber) && rowNumber >= 1) {
      onGo(Math.min(rowCount, rowNumber));
      setValue("");
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent size="sm">
        <DialogHeader>
          <DialogTitle>Go to row</DialogTitle>
        </DialogHeader>
        <DialogBody className="flex items-center gap-2">
          <Input
            // oxlint-disable-next-line jsx-a11y/no-autofocus -- single-field dialog; focus belongs in the row input.
            autoFocus
            type="number"
            min={1}
            max={rowCount}
            placeholder={`1–${rowCount}`}
            value={value}
            onChange={(event) => setValue(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submit();
              }
            }}
          />
          <Button size="sm" onClick={submit}>
            Go
          </Button>
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
