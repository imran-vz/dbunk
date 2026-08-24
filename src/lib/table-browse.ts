/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type, anti-slop/require-safety-comment-for-type-assertion -- Prefs JSON is an external boundary parsed into named types here. */
import type { DatabaseEngine } from "@/lib/store/types";

export const TABLE_BROWSE_PAGE_SIZES = [
  10, 25, 50, 100, 250, 500, 1000,
] as const;
export const DEFAULT_TABLE_BROWSE_PAGE_SIZE = 100;
export const TABLE_GRID_PREFS_HISTORY_CAP = 20;
export const TABLE_GRID_PREFS_VERSION = 1;
export const GRID_NULL_SENTINEL = "NULL";

export type ComparisonOperator = "eq" | "neq" | "lt" | "lte" | "gt" | "gte";
export type TextMatchOperator =
  | "contains"
  | "notContains"
  | "startsWith"
  | "endsWith";
export type BrowseDraftOperator =
  | ComparisonOperator
  | TextMatchOperator
  | "isNull"
  | "isNotNull"
  | "inList";

export type BrowseFilter =
  | {
      kind: "comparison";
      column: string;
      operator: ComparisonOperator;
      value: string;
    }
  | {
      kind: "textMatch";
      column: string;
      operator: TextMatchOperator;
      value: string;
    }
  | { kind: "isNull"; column: string }
  | { kind: "isNotNull"; column: string }
  | { kind: "inList"; column: string; values: string[] }
  | { kind: "rawSql"; text: string };

export type BrowseSortDirection = "asc" | "desc";
export type BrowseNulls = "default" | "first" | "last";

export type BrowseSortKey = {
  column: string;
  direction: BrowseSortDirection;
  nulls: BrowseNulls;
};

export type BrowseCursor = { values: string[] };

export type BrowsePageRequest =
  | { kind: "offset"; page: number }
  | { kind: "keyset"; cursor: BrowseCursor | null };

export type BrowseCountPolicy = "none" | "estimated";

export type BrowseIdentityKind =
  | "primaryKey"
  | "uniqueIndex"
  | "virtual"
  | "none";

export type BrowseIdentity = {
  kind: BrowseIdentityKind;
  columns: string[];
};

export type BrowseColumn = {
  name: string;
  castType: string;
  nullable: boolean;
};

export type BrowsePageMode = "offset" | "keyset";

export type BrowsePageInfo = {
  mode: BrowsePageMode;
  page: number | null;
  hasMore: boolean;
  nextCursor: BrowseCursor | null;
};

export type BrowseCountKind = "exact" | "estimated" | "unknown";

export type BrowseCount = {
  kind: BrowseCountKind;
  value: number | null;
};

export type InspectionParam =
  | { kind: "text"; value: string }
  | { kind: "textArray"; values: string[] };

export type BrowseInspection = {
  sql: string;
  params: InspectionParam[];
};

export type BrowseTableResult = {
  requestId: number;
  columns: BrowseColumn[];
  rows: Array<Array<string | null>>;
  identity: BrowseIdentity;
  rowIdentity: string[][] | null;
  pageInfo: BrowsePageInfo;
  count: BrowseCount;
  inspection: BrowseInspection;
  omittedRows: number;
  truncatedCells: number;
  runtimeMs: number;
};

export type BrowseExactCountResult = {
  kind: BrowseCountKind;
  value: number;
  requestId: number;
};

export type CancelTableBrowseResult = {
  cancelRequested: boolean;
};

export type TableBrowseError =
  | { kind: "unsupportedEngine" }
  | { kind: "unknownColumn"; column: string }
  | { kind: "invalidFilter"; reason: string }
  | { kind: "invalidSort"; column: string }
  | { kind: "invalidCursor" }
  | { kind: "superseded" }
  | { kind: "cancelled" }
  | { kind: "connectionClosing" }
  | { kind: "connectionLost" }
  | { kind: "timeout"; operation: string }
  | {
      kind: "database";
      code: string | null;
      message: string;
      severity: string | null;
      position: number | null;
    };

export type TableBrowseFilterMode = "typed" | "raw";

export type TableBrowseHistoryEntry = {
  appliedAt: string;
  typedFilters: BrowseFilter[];
  rawFilterText: string;
  filterMode: TableBrowseFilterMode;
  sort: BrowseSortKey[];
};

export type TableBrowsePreset = {
  name: string;
  typedFilters: BrowseFilter[];
  rawFilterText: string;
  filterMode: TableBrowseFilterMode;
  sort: BrowseSortKey[];
  pageSize: number;
};

export type TableGridPrefs = {
  version: number;
  pageSize: number;
  sort: BrowseSortKey[];
  typedFilters: BrowseFilter[];
  rawFilterText: string;
  filterMode: TableBrowseFilterMode;
  filterHistory: TableBrowseHistoryEntry[];
  sortHistory: TableBrowseHistoryEntry[];
  presets: TableBrowsePreset[];
  columnWidths?: Record<string, number>;
};

export type BrowseTableDataPayload = {
  connectionId: string;
  tabId: string;
  requestId: number;
  schema: string;
  table: string;
  filters: BrowseFilter[];
  sort: BrowseSortKey[];
  pageRequest: BrowsePageRequest;
  pageSize: number;
  countPolicy: BrowseCountPolicy;
  refreshStructure: boolean;
};

export type CountTableBrowseRowsPayload = {
  connectionId: string;
  tabId: string;
  requestId: number;
  schema: string;
  table: string;
  filters: BrowseFilter[];
};

export function supportsServerTableBrowse(engine: DatabaseEngine): boolean {
  return engine === "PostgreSQL";
}

export function browseCellsToGrid(
  rows: Array<Array<string | null>>,
): string[][] {
  return rows.map((row) => row.map((cell) => cell ?? GRID_NULL_SENTINEL));
}

export function gridCellToEditValue(cell: string): string | null {
  return cell === GRID_NULL_SENTINEL ? null : cell;
}

export function defaultTableGridPrefs(): TableGridPrefs {
  return {
    version: TABLE_GRID_PREFS_VERSION,
    pageSize: DEFAULT_TABLE_BROWSE_PAGE_SIZE,
    sort: [],
    typedFilters: [],
    rawFilterText: "",
    filterMode: "typed",
    filterHistory: [],
    sortHistory: [],
    presets: [],
  };
}

export function clampBrowsePageSize(pageSize: number): number {
  for (const size of TABLE_BROWSE_PAGE_SIZES) {
    if (size === pageSize) return size;
  }
  return DEFAULT_TABLE_BROWSE_PAGE_SIZE;
}

export function browseOperatorNeedsValue(
  operator: BrowseDraftOperator,
): boolean {
  return operator !== "isNull" && operator !== "isNotNull";
}

export function buildBrowseFilter(
  column: string,
  operator: BrowseDraftOperator,
  rawValue: string,
): BrowseFilter | null {
  if (column === "") return null;
  if (operator === "isNull") return { kind: "isNull", column };
  if (operator === "isNotNull") return { kind: "isNotNull", column };
  if (operator === "inList") {
    const values = rawValue
      .split(",")
      .map((part) => part.trim())
      .filter((part) => part.length > 0);
    if (values.length === 0) return null;
    return { kind: "inList", column, values };
  }
  const value = rawValue.trim();
  if (value.length === 0) return null;
  if (
    operator === "contains" ||
    operator === "notContains" ||
    operator === "startsWith" ||
    operator === "endsWith"
  ) {
    return { kind: "textMatch", column, operator, value };
  }
  return { kind: "comparison", column, operator, value };
}

export function filtersForRequest(
  typedFilters: BrowseFilter[],
  rawFilterText: string,
): BrowseFilter[] {
  const trimmed = rawFilterText.trim();
  if (trimmed.length === 0) return typedFilters;
  return [...typedFilters, { kind: "rawSql", text: trimmed }];
}

export function parseTableGridPrefs(value: unknown): TableGridPrefs {
  const defaults = defaultTableGridPrefs();
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return defaults;
  }
  const record = value as Record<string, unknown>;
  const version =
    typeof record.version === "number" && record.version >= 1
      ? record.version
      : defaults.version;
  return {
    version,
    pageSize: clampBrowsePageSize(
      typeof record.pageSize === "number" ? record.pageSize : defaults.pageSize,
    ),
    sort: Array.isArray(record.sort)
      ? record.sort.filter(isBrowseSortKey)
      : defaults.sort,
    typedFilters: Array.isArray(record.typedFilters)
      ? record.typedFilters.filter(isBrowseFilter)
      : defaults.typedFilters,
    rawFilterText:
      typeof record.rawFilterText === "string"
        ? record.rawFilterText
        : defaults.rawFilterText,
    filterMode: record.filterMode === "raw" ? "raw" : "typed",
    filterHistory: Array.isArray(record.filterHistory)
      ? record.filterHistory
          .filter(isHistoryEntry)
          .slice(0, TABLE_GRID_PREFS_HISTORY_CAP)
      : defaults.filterHistory,
    sortHistory: Array.isArray(record.sortHistory)
      ? record.sortHistory
          .filter(isHistoryEntry)
          .slice(0, TABLE_GRID_PREFS_HISTORY_CAP)
      : defaults.sortHistory,
    presets: Array.isArray(record.presets)
      ? record.presets.filter(isPreset)
      : defaults.presets,
    columnWidths:
      record.columnWidths &&
      typeof record.columnWidths === "object" &&
      !Array.isArray(record.columnWidths)
        ? Object.fromEntries(
            Object.entries(
              record.columnWidths as Record<string, unknown>,
            ).filter(
              (entry): entry is [string, number] =>
                typeof entry[1] === "number",
            ),
          )
        : undefined,
  };
}

export function pushHistory(
  entries: TableBrowseHistoryEntry[],
  next: TableBrowseHistoryEntry,
): TableBrowseHistoryEntry[] {
  return [next, ...entries].slice(0, TABLE_GRID_PREFS_HISTORY_CAP);
}

export function cycleSort(
  sort: BrowseSortKey[],
  column: string,
  append: boolean,
): BrowseSortKey[] {
  const index = sort.findIndex((key) => key.column === column);
  if (!append) {
    if (index === 0 && sort.length === 1 && sort[0]?.direction === "asc") {
      return [{ column, direction: "desc", nulls: "default" }];
    }
    if (index === 0 && sort.length === 1 && sort[0]?.direction === "desc") {
      return [];
    }
    return [{ column, direction: "asc", nulls: "default" }];
  }
  if (index < 0) {
    return [...sort, { column, direction: "asc", nulls: "default" }];
  }
  const current = sort[index];
  if (!current) return sort;
  if (current.direction === "asc") {
    return sort.map((key, keyIndex) =>
      keyIndex === index ? { ...key, direction: "desc" } : key,
    );
  }
  return sort.filter((_, keyIndex) => keyIndex !== index);
}

export function identityIsEditable(kind: BrowseIdentityKind): boolean {
  return kind === "primaryKey" || kind === "uniqueIndex";
}

export function browseIdentityReadOnlyCopy(kind: BrowseIdentityKind): string {
  if (kind === "virtual") {
    return "This table is paged with a virtual identity and is read-only.";
  }
  return "This table has no usable row identity and is read-only.";
}

function isBrowseSortKey(value: unknown): value is BrowseSortKey {
  if (value === null || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.column === "string" &&
    (record.direction === "asc" || record.direction === "desc") &&
    (record.nulls === "default" ||
      record.nulls === "first" ||
      record.nulls === "last")
  );
}

function isBrowseFilter(value: unknown): value is BrowseFilter {
  if (value === null || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  switch (record.kind) {
    case "comparison":
      return (
        typeof record.column === "string" &&
        typeof record.value === "string" &&
        (record.operator === "eq" ||
          record.operator === "neq" ||
          record.operator === "lt" ||
          record.operator === "lte" ||
          record.operator === "gt" ||
          record.operator === "gte")
      );
    case "textMatch":
      return (
        typeof record.column === "string" &&
        typeof record.value === "string" &&
        (record.operator === "contains" ||
          record.operator === "notContains" ||
          record.operator === "startsWith" ||
          record.operator === "endsWith")
      );
    case "isNull":
    case "isNotNull":
      return typeof record.column === "string";
    case "inList":
      return (
        typeof record.column === "string" &&
        Array.isArray(record.values) &&
        record.values.every((item) => typeof item === "string") &&
        record.values.length > 0
      );
    case "rawSql":
      return typeof record.text === "string";
    default:
      return false;
  }
}

function isHistoryEntry(value: unknown): value is TableBrowseHistoryEntry {
  if (value === null || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.appliedAt === "string" &&
    Array.isArray(record.typedFilters) &&
    record.typedFilters.every(isBrowseFilter) &&
    typeof record.rawFilterText === "string" &&
    (record.filterMode === "typed" || record.filterMode === "raw") &&
    Array.isArray(record.sort) &&
    record.sort.every(isBrowseSortKey)
  );
}

function isPreset(value: unknown): value is TableBrowsePreset {
  if (value === null || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return (
    typeof record.name === "string" &&
    record.name.trim().length > 0 &&
    Array.isArray(record.typedFilters) &&
    record.typedFilters.every(isBrowseFilter) &&
    typeof record.rawFilterText === "string" &&
    (record.filterMode === "typed" || record.filterMode === "raw") &&
    Array.isArray(record.sort) &&
    record.sort.every(isBrowseSortKey) &&
    typeof record.pageSize === "number"
  );
}
