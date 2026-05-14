import { describe, expect, it } from "vitest";

import type { ColumnInfo } from "@/lib/store";

import { generateMockInsertSql, splitQualifiedTable } from "./mock-data";

const col = (
  name: string,
  dataType: string,
  overrides: Partial<ColumnInfo> = {},
): ColumnInfo => ({
  name,
  dataType,
  nullable: false,
  defaultValue: null,
  isPrimaryKey: false,
  ordinalPosition: 0,
  ...overrides,
});

describe("generateMockInsertSql", () => {
  it("returns a placeholder when no columns are supplied", () => {
    expect(generateMockInsertSql("public.users", [])).toContain(
      "no column metadata",
    );
  });

  it("returns a placeholder when the table name is blank", () => {
    expect(generateMockInsertSql("   ", [col("id", "int")])).toContain(
      "target table is required",
    );
  });

  it("emits DEFAULT for primary key columns", () => {
    const sql = generateMockInsertSql(
      "public.users",
      [col("id", "bigint", { isPrimaryKey: true }), col("name", "text")],
      { rowCount: 1 },
    );
    expect(sql).toMatch(
      /INSERT INTO public.users \(id, name\) VALUES \(DEFAULT, '/,
    );
  });

  it("emits DEFAULT when the column has a server-side default", () => {
    const sql = generateMockInsertSql(
      "public.users",
      [col("created_at", "timestamptz", { defaultValue: "now()" })],
      { rowCount: 1 },
    );
    expect(sql).toContain("VALUES (DEFAULT)");
  });

  it("renders type-appropriate literals for common Postgres types", () => {
    const sql = generateMockInsertSql(
      "public.events",
      [
        col("id", "bigint", { isPrimaryKey: true }),
        col("payload", "jsonb"),
        col("tags", "text[]"),
        col("count", "integer"),
        col("ratio", "numeric"),
        col("active", "boolean"),
        col("ref", "uuid"),
        col("occurred_on", "date"),
        col("occurred_at", "timestamptz"),
        col("ip", "inet"),
        col("label", "varchar"),
        col("duration", "interval"),
      ],
      { rowCount: 1, seed: 42 },
    );
    expect(sql).toContain("'{\"sample\":1}'::jsonb");
    expect(sql).toContain("ARRAY['sample-1']");
    expect(sql).toMatch(/[0-9]+/);
    expect(sql).toContain("gen_random_uuid()");
    expect(sql).toContain("CURRENT_DATE");
    expect(sql).toContain("now()");
    expect(sql).toContain("'192.0.2.1'::inet");
    expect(sql).toContain("'Sample 1'");
    expect(sql).toMatch(/(TRUE|FALSE)/);
    expect(sql).toContain("INTERVAL '1 day'");
  });

  it("quotes identifiers that aren't simple lowercase names", () => {
    const sql = generateMockInsertSql(
      "public.users",
      [col("Name With Space", "text", { isPrimaryKey: true })],
      { rowCount: 1 },
    );
    expect(sql).toContain('("Name With Space")');
  });

  it("emits the requested number of rows", () => {
    const sql = generateMockInsertSql("public.users", [col("name", "text")], {
      rowCount: 3,
    });
    expect(sql.split("\n")).toHaveLength(3);
  });
});

describe("splitQualifiedTable", () => {
  it("parses schema.table form", () => {
    expect(splitQualifiedTable("app.users", "public")).toEqual({
      schema: "app",
      table: "users",
    });
  });

  it("falls back to the default schema for bare table names", () => {
    expect(splitQualifiedTable("users", "public")).toEqual({
      schema: "public",
      table: "users",
    });
  });

  it("returns null for empty input", () => {
    expect(splitQualifiedTable("  ", "public")).toBeNull();
  });

  it("returns null for over-qualified input", () => {
    expect(splitQualifiedTable("a.b.c", "public")).toBeNull();
  });

  it("strips wrapping double quotes", () => {
    expect(splitQualifiedTable('"app"."Users"', "public")).toEqual({
      schema: "app",
      table: "Users",
    });
  });
});
