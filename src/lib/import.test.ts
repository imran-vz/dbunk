import { describe, expect, it } from "vitest";
import {
  buildDefaultColumnMapping,
  detectCsvHeader,
  parseCsvImport,
  parseDelimited,
  parseWorkbookImport,
  shouldUseCopyFastPath,
} from "@/lib/import";

describe("parseDelimited", () => {
  it("parses quoted CSV cells and escaped quotes", () => {
    expect(parseDelimited('id,note\n1,"hello, ""Ada"""\n')).toEqual([
      ["id", "note"],
      ["1", 'hello, "Ada"'],
    ]);
  });
});

describe("import sheet parsing", () => {
  it("detects a header row when labels are followed by data", () => {
    expect(
      detectCsvHeader([
        ["id", "created_at"],
        ["1", "2026-05-13"],
      ]),
    ).toBe(true);
  });

  it("turns CSV into columns and null-aware rows", () => {
    expect(parseCsvImport("id,name\n1,Ada\n2,NULL")).toEqual({
      name: "CSV import",
      columns: ["id", "name"],
      rows: [
        ["1", "Ada"],
        ["2", null],
      ],
    });
  });

  it("supports workbook-style multi-sheet imports", () => {
    expect(
      parseWorkbookImport(
        [
          { name: "users", rows: [["id"], ["1"]] },
          { name: "orders", rows: [["id"], ["10"]] },
        ],
        { hasHeader: true },
      ).map((sheet) => sheet.name),
    ).toEqual(["users", "orders"]);
  });

  it("builds case-insensitive column mappings", () => {
    expect(
      buildDefaultColumnMapping(["ID", "name", "skip"], ["id", "name"]),
    ).toEqual([
      { sourceColumn: "ID", targetColumn: "id", include: true },
      { sourceColumn: "name", targetColumn: "name", include: true },
      { sourceColumn: "skip", targetColumn: "", include: false },
    ]);
  });

  it("selects COPY FROM only for large untransformed Postgres CSV imports", () => {
    expect(
      shouldUseCopyFastPath({
        engine: "PostgreSQL",
        format: "csv",
        rowCount: 10_000,
        hasTransform: false,
      }),
    ).toBe(true);
    expect(
      shouldUseCopyFastPath({
        engine: "PostgreSQL",
        format: "xlsx",
        rowCount: 10_000,
        hasTransform: false,
      }),
    ).toBe(false);
  });
});
