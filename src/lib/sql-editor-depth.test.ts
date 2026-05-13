import { describe, expect, it } from "vitest";
import {
  applyBindVariables,
  DEFAULT_SQL_SNIPPETS,
  explainSql,
  normalizePlanNode,
  renderSnippet,
} from "@/lib/sql-editor-depth";

describe("SQL editor depth", () => {
  it("wraps queries for EXPLAIN JSON and EXPLAIN ANALYZE", () => {
    expect(explainSql("select * from users;")).toBe(
      "EXPLAIN (VERBOSE, FORMAT JSON) select * from users;",
    );
    expect(explainSql("select * from users", "analyze")).toContain(
      "ANALYZE, BUFFERS",
    );
  });

  it("normalizes nested Postgres plan nodes", () => {
    expect(
      normalizePlanNode({
        "Node Type": "Nested Loop",
        "Total Cost": 10,
        Plans: [{ "Node Type": "Seq Scan", "Relation Name": "users" }],
      }),
    ).toEqual({
      nodeType: "Nested Loop",
      relationName: undefined,
      totalCost: 10,
      actualTotalTime: undefined,
      plans: [
        {
          nodeType: "Seq Scan",
          relationName: "users",
          totalCost: undefined,
          actualTotalTime: undefined,
          plans: [],
        },
      ],
    });
  });

  it("ships a snippets library", () => {
    expect(DEFAULT_SQL_SNIPPETS.map((snippet) => snippet.id)).toEqual(
      expect.arrayContaining([
        "select-limit",
        "count-table",
        "active-sessions",
      ]),
    );
  });

  it("applies bind variables safely", () => {
    expect(
      applyBindVariables(
        "select * from users where id = :id and active = :on",
        {
          id: 7,
          on: true,
        },
      ),
    ).toBe("select * from users where id = 7 and active = TRUE");
    expect(() => applyBindVariables("select :missing", {})).toThrow(
      "Missing bind variable :missing",
    );
  });

  it("renders template snippets", () => {
    expect(
      renderSnippet(DEFAULT_SQL_SNIPPETS[0], {
        schema: "public",
        table: "users",
        limit: 50,
      }),
    ).toBe("SELECT * FROM public.users LIMIT 50;");
  });
});
