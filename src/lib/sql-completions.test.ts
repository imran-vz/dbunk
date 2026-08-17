import { describe, expect, it } from "vitest";

import { getSqlCompletions } from "@/lib/sql-completions";
import { tableStructureKey } from "@/lib/store";

const schemas = [
  {
    name: "public",
    tables: ["users", "orders"],
    views: ["active_users"],
  },
  {
    name: "analytics",
    tables: ["events"],
    views: [],
  },
];

describe("getSqlCompletions", () => {
  it("prioritizes current-schema tables after FROM", () => {
    const completions = getSqlCompletions("select * from ", {
      schemas,
      currentSchema: "public",
    });

    expect(completions.slice(0, 3).map((item) => item.label)).toEqual([
      "users",
      "orders",
      "active_users",
    ]);
    expect(
      completions.find((item) => item.label === "analytics.events"),
    ).toMatchObject({
      insertText: "analytics.events",
      kind: "table",
    });
  });

  it("suggests tables for a qualified schema", () => {
    const completions = getSqlCompletions("select * from analytics.", {
      schemas,
      currentSchema: "public",
    });

    expect(completions.map((item) => item.label)).toEqual(["events"]);
    expect(completions[0]).toMatchObject({
      insertText: "events",
      detail: "Table in analytics",
    });
  });

  it("still offers SQL syntax when not in a table position", () => {
    const completions = getSqlCompletions("select * wh", {
      schemas,
      currentSchema: "public",
    });

    expect(completions[0]).toMatchObject({
      label: "select",
      kind: "keyword",
    });
    expect(completions.some((item) => item.label === "where")).toBe(true);
  });

  it("quotes identifiers that need escaping", () => {
    const completions = getSqlCompletions("select * from ", {
      schemas: [{ name: "app data", tables: ["order items"], views: [] }],
      currentSchema: "app data",
    });

    expect(completions[0]).toMatchObject({
      label: "order items",
      insertText: '"order items"',
    });
  });

  it("suggests columns from the current table in a where clause", () => {
    const completions = getSqlCompletions(
      "select * from public.session_state where ",
      {
        connectionId: "conn-1",
        schemas,
        currentSchema: "public",
        tableStructure: {
          [tableStructureKey("conn-1", "public", "session_state")]: {
            columns: [
              {
                name: "id",
                dataType: "uuid",
                nullable: false,
                defaultValue: null,
                isPrimaryKey: true,
                ordinalPosition: 1,
              },
              {
                name: "updated_at",
                dataType: "timestamp with time zone",
                nullable: false,
                defaultValue: null,
                isPrimaryKey: false,
                ordinalPosition: 2,
              },
            ],
            primaryKey: ["id"],
            foreignKeys: [],
            indexes: [],
            constraints: [],
            capabilities: {
              columns: true,
              primaryKey: true,
              foreignKeys: true,
              indexes: true,
              constraints: true,
              canInsertRows: true,
              canUpdateRows: true,
              canDeleteRows: true,
              canAlterSchema: true,
              uniquenessGuarantee: "exact",
            },
          },
        },
      },
    );

    expect(completions.slice(0, 2).map((item) => item.label)).toEqual([
      "id",
      "updated_at",
    ]);
    expect(completions[0]).toMatchObject({
      kind: "column",
      detail: "uuid primary key in public.session_state",
    });
  });
});
