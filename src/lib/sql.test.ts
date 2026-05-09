import { describe, expect, it } from "vitest";
import { pickSqlToRun } from "@/lib/sql";

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
