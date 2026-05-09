import { describe, expect, it } from "vitest";
import {
  getSqlStatementAtPosition,
  getSqlStatements,
  pickSqlToRun,
} from "@/lib/sql";

describe("pickSqlToRun", () => {
  it("returns the selection when it has non-whitespace content", () => {
    expect(pickSqlToRun("select * from users;", "select 1")).toBe("select 1");
  });

  it("preserves leading/trailing whitespace inside a non-empty selection", () => {
    expect(pickSqlToRun("full text", "  select 1  ")).toBe("  select 1  ");
  });

  it("falls back to the full text when the selection is null", () => {
    expect(pickSqlToRun("select * from users;", null)).toBe(
      "select * from users;",
    );
  });

  it("falls back to the full text when the selection is an empty string", () => {
    expect(pickSqlToRun("select * from users;", "")).toBe(
      "select * from users;",
    );
  });

  it("falls back to the full text when the selection is whitespace only", () => {
    expect(pickSqlToRun("select * from users;", "   \n\t  ")).toBe(
      "select * from users;",
    );
  });

  it("returns the full text unchanged even when it has only whitespace", () => {
    expect(pickSqlToRun("   ", null)).toBe("   ");
  });
});

describe("getSqlStatements", () => {
  it("splits statements on semicolons", () => {
    expect(
      getSqlStatements("select 1;\nselect 2;").map((item) => item.sql),
    ).toEqual(["select 1", "select 2"]);
  });

  it("keeps semicolons inside strings and comments", () => {
    expect(
      getSqlStatements("select ';'; -- ;\nselect 2;").map((item) => item.sql),
    ).toEqual(["select ';'", "-- ;\nselect 2"]);
  });

  it("returns a multiline statement at the cursor position", () => {
    const sql = "select *\nfrom users\nwhere id = 1;\nselect 2;";

    expect(getSqlStatementAtPosition(sql, 2, 4)?.sql).toBe(
      "select *\nfrom users\nwhere id = 1",
    );
    expect(getSqlStatementAtPosition(sql, 4, 4)?.sql).toBe("select 2");
  });

  it("returns null when the cursor is outside any non-empty statement", () => {
    expect(getSqlStatementAtPosition("\n\n", 1, 1)).toBeNull();
  });
});
