/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- persisted grid-layout state is an external boundary validated field-by-field in this module. */
/**
 * Pure model helpers for the virtualized data grid (DESIGN-SYSTEM
 * §5.4): fixed-size virtual ranges, content-derived column widths,
 * cell alignment detection, per-table layout persistence, and the
 * copy-as text builders shared with the context menu (and the P7
 * contextual palette).
 */

import {
  type ExportTable,
  toCsv,
  toJson,
  toMarkdown,
  toSqlInserts,
} from "@/lib/export";
import { GRID_NULL_SENTINEL } from "@/lib/table-browse";
import { uiGet, uiSet } from "@/lib/ui-state";

// ---------------------------------------------------------------------------
// Virtual ranges (fixed row height; variable column widths)
// ---------------------------------------------------------------------------

export type VirtualRange = { start: number; end: number };

/** Visible index range for fixed-size items, inclusive start / exclusive end. */
export function virtualRowRange(
  scrollTop: number,
  viewportHeight: number,
  rowCount: number,
  rowHeight: number,
  overscan = 8,
): VirtualRange {
  if (rowCount === 0 || rowHeight <= 0) return { start: 0, end: 0 };
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(
    rowCount,
    Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan,
  );
  return { start, end };
}

/**
 * Visible column range against a prefix-sum offset table.
 * `offsets[i]` is the left edge of column i; `offsets[count]` the total.
 */
export function virtualColumnRange(
  scrollLeft: number,
  viewportWidth: number,
  offsets: number[],
  overscan = 3,
): VirtualRange {
  const count = offsets.length - 1;
  if (count <= 0) return { start: 0, end: 0 };
  let low = 0;
  let high = count - 1;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (offsets[mid + 1] <= scrollLeft) low = mid + 1;
    else high = mid;
  }
  const start = Math.max(0, low - overscan);
  let end = low;
  const rightEdge = scrollLeft + viewportWidth;
  while (end < count && offsets[end] < rightEdge) end += 1;
  return { start, end: Math.min(count, end + overscan) };
}

export function columnOffsets(widths: number[]): number[] {
  const offsets = [0];
  for (let index = 0; index < widths.length; index += 1) {
    offsets.push(offsets[index] + widths[index]);
  }
  return offsets;
}

// ---------------------------------------------------------------------------
// Column widths
// ---------------------------------------------------------------------------

/** Approximate advance width of one mono-grid character at 12px. */
const CHAR_WIDTH = 7.3;
/** Cell horizontal padding (8px each side) + border allowance. */
const CELL_CHROME = 18;
export const MIN_COLUMN_WIDTH = 60;
/** Initial content-derived clamp (§5.4). */
export const MAX_INITIAL_COLUMN_WIDTH = 400;
/** Auto-fit clamp (§5.4). */
export const MAX_AUTO_FIT_WIDTH = 500;

const WIDTH_SAMPLE_ROWS = 100;

const clampWidth = (width: number, max: number): number =>
  Math.round(Math.min(max, Math.max(MIN_COLUMN_WIDTH, width)));

/**
 * Content-derived initial widths: sample the first rows, size to the
 * longest of header and sampled content, clamp 60–400 (§5.4).
 */
export function estimateInitialWidths(
  columns: string[],
  rows: ReadonlyArray<ReadonlyArray<string>>,
): number[] {
  const sample = rows.slice(0, WIDTH_SAMPLE_ROWS);
  return columns.map((column, index) => {
    // Header renders 13px sans + role icons; give it a slight factor.
    let longest = column.length + 3;
    for (const row of sample) {
      const cell = row[index];
      if (!cell) continue;
      const newline = cell.indexOf("\n");
      const length = newline === -1 ? cell.length : newline + 2;
      if (length > longest) longest = length;
    }
    return clampWidth(
      longest * CHAR_WIDTH + CELL_CHROME,
      MAX_INITIAL_COLUMN_WIDTH,
    );
  });
}

/** Double-click auto-fit: size to loaded content, clamp 500 (§5.4). */
export function autoFitWidth(
  column: string,
  columnIndex: number,
  rows: ReadonlyArray<ReadonlyArray<string>>,
): number {
  let longest = column.length + 3;
  for (const row of rows) {
    const cell = row[columnIndex];
    if (!cell) continue;
    const newline = cell.indexOf("\n");
    const length = newline === -1 ? cell.length : newline + 2;
    if (length > longest) longest = length;
  }
  return clampWidth(longest * CHAR_WIDTH + CELL_CHROME, MAX_AUTO_FIT_WIDTH);
}

// ---------------------------------------------------------------------------
// Cell alignment
// ---------------------------------------------------------------------------

export type CellAlignment = "left" | "right" | "center";

const NUMERIC_TYPE = /int|serial|numeric|decimal|float|double|real|money/i;
const BOOLEAN_TYPE = /^bool/i;
const NUMERIC_VALUE = /^-?(\d[\d_]*)(\.\d+)?([eE][+-]?\d+)?$/;

/**
 * Numbers right-align with `tnum`; booleans center; text stays left
 * (§5.4). Prefers declared column types; falls back to sampling when
 * type info is absent (ad-hoc query results).
 */
export function detectAlignment(
  columnType: string | undefined,
  columnIndex: number,
  rows: ReadonlyArray<ReadonlyArray<string>>,
): CellAlignment {
  if (columnType) {
    if (BOOLEAN_TYPE.test(columnType)) return "center";
    return NUMERIC_TYPE.test(columnType) ? "right" : "left";
  }
  let seen = 0;
  let numeric = 0;
  for (const row of rows.slice(0, WIDTH_SAMPLE_ROWS)) {
    const cell = row[columnIndex];
    if (!cell || cell === GRID_NULL_SENTINEL) continue;
    seen += 1;
    if (NUMERIC_VALUE.test(cell)) numeric += 1;
  }
  return seen > 0 && numeric === seen ? "right" : "left";
}

// ---------------------------------------------------------------------------
// Per-table layout persistence (P8 UI-state store)
// ---------------------------------------------------------------------------

export type GridLayoutState = {
  widths: Record<string, number>;
  pinned: string[];
};

const LAYOUT_STORAGE_PREFIX = "dbunk.grid.layout.";

export function loadGridLayout(key: string | undefined): GridLayoutState {
  const empty: GridLayoutState = { widths: {}, pinned: [] };
  if (!key) return empty;
  try {
    const raw = uiGet(`${LAYOUT_STORAGE_PREFIX}${key}`);
    if (!raw) return empty;
    // SAFETY: persisted layout is validated field-by-field below.
    const parsed = JSON.parse(raw) as Partial<GridLayoutState>;
    const widths: Record<string, number> = {};
    if (parsed.widths && typeof parsed.widths === "object") {
      for (const [column, width] of Object.entries(parsed.widths)) {
        if (typeof width === "number" && Number.isFinite(width)) {
          widths[column] = clampWidth(width, MAX_AUTO_FIT_WIDTH);
        }
      }
    }
    const isString = (value: unknown): value is string =>
      typeof value === "string";
    const pinned = Array.isArray(parsed.pinned)
      ? parsed.pinned.filter(isString)
      : [];
    return { widths, pinned };
  } catch {
    return empty;
  }
}

export function saveGridLayout(
  key: string | undefined,
  state: GridLayoutState,
): void {
  if (!key) return;
  uiSet(`${LAYOUT_STORAGE_PREFIX}${key}`, JSON.stringify(state));
}

// ---------------------------------------------------------------------------
// Copy-as builders (§5.4 — shared by Cmd+C, context menu, P7 palette)
// ---------------------------------------------------------------------------

export type CopyFormat = "tsv" | "csv" | "json" | "insert" | "markdown";

export const COPY_FORMATS: ReadonlyArray<{ id: CopyFormat; label: string }> = [
  { id: "tsv", label: "TSV" },
  { id: "csv", label: "CSV" },
  { id: "json", label: "JSON" },
  { id: "insert", label: "INSERT" },
  { id: "markdown", label: "Markdown" },
];

export function buildCopyText(
  format: CopyFormat,
  table: ExportTable,
  tableName: string,
): string {
  switch (format) {
    // Plain Cmd+C TSV pastes into spreadsheets/editors — no header row
    // (DataGrip convention); explicit formats include headers.
    case "tsv":
      return table.rows
        .map((row) =>
          row
            .map((cell) =>
              cell === null
                ? GRID_NULL_SENTINEL
                : cell.replace(/\t/g, " ").replace(/\r?\n/g, " "),
            )
            .join("\t"),
        )
        .join("\n");
    case "csv":
      return toCsv(table, { nullAs: GRID_NULL_SENTINEL });
    case "json":
      return toJson(table, { pretty: true });
    case "insert":
      return toSqlInserts(table, { tableName: tableName || "table_name" });
    case "markdown":
      return toMarkdown(table, GRID_NULL_SENTINEL);
  }
}
