import { describe, expect, it } from "vitest";
import {
  buildImportRows,
  defaultColumnMapping,
  detectHeader,
  normalizeRows,
  parseCsv,
  shouldUseCopyFrom,
} from "@/lib/import";

describe("data import parsing", () => {
  it("parses quoted CSV cells", () => {
    expect(parseCsv('id,note\n1,"hello, ""Ada"""')).toEqual([
      ["id", "note"],
      ["1", 'hello, "Ada"'],
    ]);
  });

  it("detects headers and normalizes rows", () => {
    const rows = [
      ["id", "name"],
      ["1", "Ada"],
    ];
    expect(detectHeader(rows)).toBe(true);
    expect(normalizeRows("users", rows)).toEqual({
      name: "users",
      columns: ["id", "name"],
      rows: [["1", "Ada"]],
    });
  });

  it("maps source columns to target columns and applies null tokens", () => {
    const sheet = normalizeRows("users", [
      ["id", "name"],
      ["1", "\\N"],
    ]);
    const mapping = defaultColumnMapping(sheet.columns, ["id", "name"]);
    expect(buildImportRows({ sheet, mapping, nullToken: "\\N" })).toEqual([
      [
        { column: "id", value: "1" },
        { column: "name", value: null },
      ],
    ]);
  });

  it("selects COPY FROM for large unmapped Postgres CSV imports", () => {
    expect(
      shouldUseCopyFrom({
        engine: "PostgreSQL",
        fileKind: "csv",
        rowCount: 10_000,
        mappingChanged: false,
      }),
    ).toBe(true);
    expect(
      shouldUseCopyFrom({
        engine: "PostgreSQL",
        fileKind: "xlsx",
        rowCount: 10_000,
        mappingChanged: false,
      }),
    ).toBe(false);
  });
});
