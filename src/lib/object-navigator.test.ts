import { describe, expect, it } from "vitest";
import {
  postgresNavigatorTemplate,
  refreshMaterializedViewSql,
  sequenceActionSql,
  tableChildNodes,
} from "@/lib/object-navigator";

describe("Postgres object navigator depth", () => {
  it("includes the planned object groups", () => {
    const kinds = postgresNavigatorTemplate().flatMap((group) =>
      (group.children ?? []).map((child) => child.kind),
    );
    expect(kinds).toEqual(
      expect.arrayContaining([
        "materialized-view",
        "function",
        "procedure",
        "aggregate",
        "sequence",
        "foreign-table",
        "type",
        "domain",
        "extension",
        "event-trigger",
        "role",
        "tablespace",
      ]),
    );
  });

  it("creates per-table child nodes", () => {
    expect(tableChildNodes("public", "users").map((node) => node.kind)).toEqual(
      [
        "table-triggers",
        "table-rules",
        "table-policies",
        "table-partitions",
        "table-dependencies",
        "table-references",
      ],
    );
  });

  it("builds sequence edit actions", () => {
    expect(
      sequenceActionSql({
        schema: "public",
        sequence: "users_id_seq",
        action: "restart",
        restartWith: 42,
      }),
    ).toBe('ALTER SEQUENCE "public"."users_id_seq" RESTART WITH 42;');
    expect(
      sequenceActionSql({
        schema: "public",
        sequence: "users_id_seq",
        action: "next-value",
      }),
    ).toBe("SELECT nextval('public.users_id_seq'::regclass);");
  });

  it("builds materialized view refresh SQL", () => {
    expect(
      refreshMaterializedViewSql({
        schema: "public",
        view: "daily_totals",
        concurrently: true,
      }),
    ).toBe('REFRESH MATERIALIZED VIEW CONCURRENTLY "public"."daily_totals";');
  });
});
