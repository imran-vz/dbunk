// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";

import {
  autoFitWidth,
  buildCopyText,
  columnOffsets,
  detectAlignment,
  estimateInitialWidths,
  loadGridLayout,
  MAX_AUTO_FIT_WIDTH,
  MAX_INITIAL_COLUMN_WIDTH,
  MIN_COLUMN_WIDTH,
  saveGridLayout,
  virtualColumnRange,
  virtualRowRange,
} from "@/components/data-grid/grid-model";

describe("virtual ranges", () => {
  it("computes the visible row window with overscan", () => {
    const range = virtualRowRange(2600, 520, 100_000, 26, 8);
    // 2600/26 = row 100; viewport shows 20 rows.
    expect(range.start).toBe(92);
    expect(range.end).toBe(128);
  });

  it("clamps to the row count", () => {
    expect(virtualRowRange(0, 600, 3, 26)).toEqual({ start: 0, end: 3 });
    expect(virtualRowRange(0, 600, 0, 26)).toEqual({ start: 0, end: 0 });
  });

  it("finds visible columns via the offset table", () => {
    const offsets = columnOffsets([100, 100, 100, 100, 100]);
    expect(offsets).toEqual([0, 100, 200, 300, 400, 500]);
    const range = virtualColumnRange(150, 200, offsets, 0);
    // 150–350px shows columns 1..3 (exclusive end).
    expect(range.start).toBe(1);
    expect(range.end).toBe(4);
  });
});

describe("column widths", () => {
  it("derives initial widths from content and clamps 60–400", () => {
    const [tiny, long] = estimateInitialWidths(
      ["a", "b"],
      [
        ["x", "y".repeat(500)],
        ["x", "z"],
      ],
    );
    expect(tiny).toBe(MIN_COLUMN_WIDTH);
    expect(long).toBe(MAX_INITIAL_COLUMN_WIDTH);
  });

  it("auto-fit clamps at 500", () => {
    expect(autoFitWidth("col", 0, [["w".repeat(1000)]])).toBe(
      MAX_AUTO_FIT_WIDTH,
    );
    expect(autoFitWidth("c", 0, [["ab"]])).toBe(MIN_COLUMN_WIDTH);
  });
});

describe("alignment detection", () => {
  it("prefers declared types", () => {
    expect(detectAlignment("int4", 0, [])).toBe("right");
    expect(detectAlignment("numeric(10,2)", 0, [])).toBe("right");
    expect(detectAlignment("boolean", 0, [])).toBe("center");
    expect(detectAlignment("text", 0, [])).toBe("left");
  });

  it("samples values when the type is unknown", () => {
    expect(detectAlignment(undefined, 0, [["1"], ["2.5"], ["NULL"]])).toBe(
      "right",
    );
    expect(detectAlignment(undefined, 0, [["1"], ["abc"]])).toBe("left");
    expect(detectAlignment(undefined, 0, [["NULL"]])).toBe("left");
  });
});

describe("grid layout persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips widths and pinned columns per key", () => {
    saveGridLayout("conn.public.users", {
      widths: { id: 80, email: 240 },
      pinned: ["id"],
    });
    const loaded = loadGridLayout("conn.public.users");
    expect(loaded.widths).toEqual({ id: 80, email: 240 });
    expect(loaded.pinned).toEqual(["id"]);
    // Another table's layout is independent.
    expect(loadGridLayout("conn.public.orders")).toEqual({
      widths: {},
      pinned: [],
    });
  });

  it("falls back to defaults on corrupt data", () => {
    window.localStorage.setItem("dbunk.grid.layout.bad", "{not json");
    expect(loadGridLayout("bad")).toEqual({ widths: {}, pinned: [] });
    window.localStorage.setItem(
      "dbunk.grid.layout.mixed",
      JSON.stringify({ widths: { a: "wide", b: 120 }, pinned: [1, "ok"] }),
    );
    expect(loadGridLayout("mixed")).toEqual({
      widths: { b: 120 },
      pinned: ["ok"],
    });
  });
});

describe("copy builders", () => {
  const table = {
    columns: ["id", "name"],
    rows: [
      ["1", "Ada"],
      ["2", null],
    ],
  };

  it("builds TSV without a header row (plain Cmd+C)", () => {
    expect(buildCopyText("tsv", table, "t")).toBe("1\tAda\n2\tNULL");
  });

  it("builds CSV with headers", () => {
    expect(buildCopyText("csv", table, "t")).toBe("id,name\n1,Ada\n2,NULL");
  });

  it("builds INSERT statements with real NULLs", () => {
    const sql = buildCopyText("insert", table, "users");
    expect(sql).toContain('INSERT INTO "users" ("id", "name")');
    expect(sql).toContain("('2', NULL);");
  });

  it("builds JSON preserving nulls", () => {
    const parsed = JSON.parse(buildCopyText("json", table, "t"));
    expect(parsed).toEqual([
      { id: "1", name: "Ada" },
      { id: "2", name: null },
    ]);
  });

  it("builds Markdown", () => {
    expect(buildCopyText("markdown", table, "t")).toContain("| id | name |");
  });
});
