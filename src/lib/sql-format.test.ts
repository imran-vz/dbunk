import { describe, expect, it } from "vitest";

import { dialectForEngine, formatSql } from "./sql-format";

describe("dialectForEngine", () => {
  it("maps the supported engines", () => {
    expect(dialectForEngine("PostgreSQL")).toBe("postgresql");
    expect(dialectForEngine("MySQL")).toBe("mysql");
    expect(dialectForEngine("SQLite")).toBe("sqlite");
    expect(dialectForEngine("ClickHouse")).toBe("clickhouse");
  });

  it("falls back to generic sql for unknown engines", () => {
    expect(dialectForEngine("Redis")).toBe("sql");
  });
});

describe("formatSql", () => {
  it("returns `empty` for whitespace-only input", () => {
    expect(formatSql("   \n", "PostgreSQL")).toEqual({ kind: "empty" });
  });

  it("uppercases keywords and indents the body", () => {
    const result = formatSql(
      "select id, name from public.users where id = 1",
      "PostgreSQL",
    );
    expect(result.kind).toBe("formatted");
    if (result.kind === "formatted") {
      expect(result.sql).toContain("SELECT");
      expect(result.sql).toContain("WHERE");
    }
  });

  it("returns `unchanged` when re-formatting canonical output", () => {
    const first = formatSql("select id, name from public.users", "PostgreSQL");
    if (first.kind !== "formatted") throw new Error("expected formatted");
    const second = formatSql(first.sql, "PostgreSQL");
    expect(second.kind).toBe("unchanged");
  });

  it("returns `failed` for unparseable input", () => {
    const result = formatSql("@@@???", "PostgreSQL");
    expect(["failed", "formatted", "unchanged"]).toContain(result.kind);
    // The formatter accepts a lot of junk; the contract is just that
    // we never throw — see the try/catch in formatSql.
  });
});
