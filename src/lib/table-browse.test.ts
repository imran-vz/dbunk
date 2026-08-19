import { describe, expect, it } from "vitest";

import type { DatabaseEngine } from "@/lib/store/types";
import {
  type BrowseFilter,
  type BrowseIdentityKind,
  type TableBrowseHistoryEntry,
  browseCellsToGrid,
  browseIdentityReadOnlyCopy,
  browseOperatorNeedsValue,
  buildBrowseFilter,
  filtersForRequest,
  GRID_NULL_SENTINEL,
  gridCellToEditValue,
  identityIsEditable,
  parseTableGridPrefs,
  pushHistory,
  supportsServerTableBrowse,
  TABLE_GRID_PREFS_HISTORY_CAP,
  TABLE_GRID_PREFS_VERSION,
} from "@/lib/table-browse";

const engines: DatabaseEngine[] = [
  "PostgreSQL",
  "MySQL",
  "ClickHouse",
  "SQLite",
  "Redis",
];

const historyEntry = (index: number): TableBrowseHistoryEntry => ({
  appliedAt: `2026-01-01T00:00:${String(index).padStart(2, "0")}.000Z`,
  typedFilters: [
    { kind: "comparison", column: "id", operator: "eq", value: String(index) },
  ],
  rawFilterText: "",
  filterMode: "typed",
  sort: [{ column: "id", direction: "asc", nulls: "default" }],
});

describe("supportsServerTableBrowse", () => {
  it("is postgres-only", () => {
    expect(engines.filter(supportsServerTableBrowse)).toEqual(["PostgreSQL"]);
    for (const engine of engines) {
      expect(supportsServerTableBrowse(engine)).toBe(engine === "PostgreSQL");
    }
  });
});

describe("buildBrowseFilter", () => {
  it("builds null operators without a value", () => {
    expect(browseOperatorNeedsValue("isNull")).toBe(false);
    expect(browseOperatorNeedsValue("eq")).toBe(true);
    expect(buildBrowseFilter("email", "isNull", "")).toEqual({
      kind: "isNull",
      column: "email",
    });
    expect(buildBrowseFilter("email", "isNotNull", "ignored")).toEqual({
      kind: "isNotNull",
      column: "email",
    });
  });

  it("rejects empty comparison values and empty in-list tokens", () => {
    expect(buildBrowseFilter("id", "eq", "  ")).toBeNull();
    expect(buildBrowseFilter("id", "inList", " , ")).toBeNull();
    expect(buildBrowseFilter("", "isNull", "")).toBeNull();
    expect(buildBrowseFilter("id", "inList", "a, b")).toEqual({
      kind: "inList",
      column: "id",
      values: ["a", "b"],
    });
  });
});

describe("browseCellsToGrid / gridCellToEditValue", () => {
  it("converts null cells to the grid sentinel and back", () => {
    expect(
      browseCellsToGrid([
        ["1", null],
        [null, "ada"],
      ]),
    ).toEqual([
      ["1", GRID_NULL_SENTINEL],
      [GRID_NULL_SENTINEL, "ada"],
    ]);
    expect(gridCellToEditValue(GRID_NULL_SENTINEL)).toBeNull();
    expect(gridCellToEditValue("ada")).toBe("ada");
  });
});

describe("parseTableGridPrefs", () => {
  it("returns defaults for missing or non-object values", () => {
    const defaults = parseTableGridPrefs(null);
    expect(defaults.version).toBe(TABLE_GRID_PREFS_VERSION);
    expect(defaults.pageSize).toBe(100);
    expect(defaults.sort).toEqual([]);
    expect(defaults.typedFilters).toEqual([]);
    expect(defaults.rawFilterText).toBe("");
    expect(defaults.filterMode).toBe("typed");
    expect(defaults.filterHistory).toEqual([]);
    expect(defaults.sortHistory).toEqual([]);
    expect(defaults.presets).toEqual([]);
    expect(parseTableGridPrefs(undefined)).toEqual(defaults);
    expect(parseTableGridPrefs("prefs")).toEqual(defaults);
    expect(parseTableGridPrefs([])).toEqual(defaults);
  });

  it("keeps a valid version and falls back when version is missing or below 1", () => {
    expect(parseTableGridPrefs({ version: 2 }).version).toBe(2);
    expect(parseTableGridPrefs({ version: 1 }).version).toBe(1);
    expect(parseTableGridPrefs({ version: 0 }).version).toBe(
      TABLE_GRID_PREFS_VERSION,
    );
    expect(parseTableGridPrefs({}).version).toBe(TABLE_GRID_PREFS_VERSION);
  });

  it("caps filter and sort history at 20", () => {
    const extra = Array.from({ length: 25 }, (_, index) => historyEntry(index));
    const parsed = parseTableGridPrefs({
      version: 1,
      filterHistory: extra,
      sortHistory: extra,
    });
    expect(parsed.filterHistory).toHaveLength(TABLE_GRID_PREFS_HISTORY_CAP);
    expect(parsed.sortHistory).toHaveLength(TABLE_GRID_PREFS_HISTORY_CAP);
    expect(parsed.filterHistory[0]?.typedFilters[0]).toEqual({
      kind: "comparison",
      column: "id",
      operator: "eq",
      value: "0",
    });
    expect(parsed.filterHistory.at(-1)?.typedFilters[0]).toEqual({
      kind: "comparison",
      column: "id",
      operator: "eq",
      value: "19",
    });
  });

  it("clamps pageSize to the allowed set", () => {
    expect(parseTableGridPrefs({ pageSize: 25 }).pageSize).toBe(25);
    expect(parseTableGridPrefs({ pageSize: 1000 }).pageSize).toBe(1000);
    expect(parseTableGridPrefs({ pageSize: 13 }).pageSize).toBe(100);
    expect(parseTableGridPrefs({ pageSize: 0 }).pageSize).toBe(100);
    expect(parseTableGridPrefs({ pageSize: 50.5 }).pageSize).toBe(100);
  });
});

describe("filtersForRequest", () => {
  it("AND-combines a raw fragment with typed filters", () => {
    const typed: BrowseFilter[] = [
      { kind: "isNotNull", column: "email" },
      { kind: "comparison", column: "id", operator: "gt", value: "10" },
    ];
    expect(filtersForRequest(typed, "")).toEqual(typed);
    expect(filtersForRequest(typed, "   ")).toEqual(typed);
    expect(filtersForRequest(typed, "  age > 3  ")).toEqual([
      ...typed,
      { kind: "rawSql", text: "age > 3" },
    ]);
  });
});

describe("pushHistory", () => {
  it("prepends and caps at 20", () => {
    let entries: TableBrowseHistoryEntry[] = [];
    for (let index = 0; index < 21; index += 1) {
      entries = pushHistory(entries, historyEntry(index));
    }
    expect(entries).toHaveLength(TABLE_GRID_PREFS_HISTORY_CAP);
    expect(entries[0]).toEqual(historyEntry(20));
    expect(entries.at(-1)).toEqual(historyEntry(1));
  });
});

describe("identityIsEditable / browseIdentityReadOnlyCopy", () => {
  it("treats primary-key and unique-index identities as editable", () => {
    const kinds: BrowseIdentityKind[] = [
      "primaryKey",
      "uniqueIndex",
      "virtual",
      "none",
    ];
    expect(kinds.filter(identityIsEditable)).toEqual([
      "primaryKey",
      "uniqueIndex",
    ]);
  });

  it("explains why virtual and missing identities are read-only", () => {
    expect(browseIdentityReadOnlyCopy("virtual")).toBe(
      "This table is paged with a virtual identity and is read-only.",
    );
    expect(browseIdentityReadOnlyCopy("none")).toBe(
      "This table has no usable row identity and is read-only.",
    );
  });
});
