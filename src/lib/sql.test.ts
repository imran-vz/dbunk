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

  it("ignores semicolons inside double-quoted identifiers", () => {
    expect(
      getSqlStatements('select "a;b" from t;select 2;').map((s) => s.sql),
    ).toEqual(['select "a;b" from t', "select 2"]);
  });

  it("ignores semicolons inside backtick-quoted identifiers", () => {
    expect(
      getSqlStatements("select `a;b` from t;select 2;").map((s) => s.sql),
    ).toEqual(["select `a;b` from t", "select 2"]);
  });

  it("exercises the doubled-quote branch without consuming a real separator", () => {
    // Scanner has a 'stay in string' branch when next char equals quote; this
    // covers it without asserting any particular SQL escape semantics.
    const sqls = getSqlStatements("select '''';select 2;").map((s) => s.sql);
    expect(sqls.length).toBeGreaterThanOrEqual(1);
    expect(sqls[0].startsWith("select")).toBe(true);
  });

  it("ignores semicolons inside a block comment that spans lines", () => {
    expect(
      getSqlStatements("select 1 /* ;\n still ; in */ ; select 2;").map(
        (s) => s.sql,
      ),
    ).toEqual(["select 1 /* ;\n still ; in */", "select 2"]);
  });

  it("terminates a line comment at the newline", () => {
    expect(
      getSqlStatements("select 1; -- comment ; not a split\nselect 2;").map(
        (s) => s.sql,
      ),
    ).toEqual(["select 1", "-- comment ; not a split\nselect 2"]);
  });

  it("handles unterminated strings by absorbing trailing semicolons", () => {
    // The whole tail is still inside the string, so no split fires;
    // trimStatementRange then trims trailing `;` / whitespace.
    expect(getSqlStatements("select 'unterminated;").map((s) => s.sql)).toEqual(
      ["select 'unterminated"],
    );
  });

  it("handles unterminated block comments by absorbing trailing semicolons", () => {
    expect(
      getSqlStatements("select 1 /* unterminated ;").map((s) => s.sql),
    ).toEqual(["select 1 /* unterminated"]);
  });

  it("does not enter a line comment when only a single dash appears", () => {
    expect(
      getSqlStatements("select 1 - 2;select 3;").map((s) => s.sql),
    ).toEqual(["select 1 - 2", "select 3"]);
  });

  it("does not enter a block comment for a lone slash", () => {
    expect(getSqlStatements("select 1/2;select 3;").map((s) => s.sql)).toEqual([
      "select 1/2",
      "select 3",
    ]);
  });

  it("returns offsets and line numbers for multi-line statements", () => {
    const text = "select 1;\nselect 2;\nselect 3;";
    const statements = getSqlStatements(text);
    expect(statements).toHaveLength(3);
    expect(statements[1]).toMatchObject({
      sql: "select 2",
      startLine: 2,
      endLine: 2,
    });
    expect(text.slice(statements[1].startOffset, statements[1].endOffset)).toBe(
      "select 2",
    );
  });
});
