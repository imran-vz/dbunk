import { describe, expect, it } from "vitest";
import {
  createForeignKeySql,
  createIndexSql,
  createTriggerSql,
  formatJsonCell,
  geometryPreviewSql,
  grantSql,
  postgresArrayLiteral,
  rlsPolicySql,
} from "@/lib/specialized-editors";

describe("specialized editors", () => {
  it("builds GRANT and RLS policy statements", () => {
    expect(
      grantSql({
        privileges: ["SELECT", "UPDATE"],
        schema: "public",
        table: "users",
        role: "analyst",
      }),
    ).toBe('GRANT SELECT, UPDATE ON TABLE "public"."users" TO "analyst";');
    expect(
      rlsPolicySql({
        schema: "public",
        table: "users",
        name: "tenant_only",
        command: "SELECT",
        using: "tenant_id = current_setting('app.tenant_id')::uuid",
      }),
    ).toContain('CREATE POLICY "tenant_only" ON "public"."users"');
  });

  it("builds index, foreign key, and trigger DDL", () => {
    expect(
      createIndexSql({
        schema: "public",
        table: "users",
        name: "users_email_idx",
        columns: ["email"],
        unique: true,
      }),
    ).toBe(
      'CREATE UNIQUE INDEX "users_email_idx" ON "public"."users" ("email");',
    );
    expect(
      createForeignKeySql({
        schema: "public",
        table: "orders",
        name: "orders_user_id_fkey",
        columns: ["user_id"],
        referencedSchema: "public",
        referencedTable: "users",
        referencedColumns: ["id"],
        onDelete: "CASCADE",
      }),
    ).toContain("ON DELETE CASCADE");
    expect(
      createTriggerSql({
        schema: "public",
        table: "users",
        name: "users_updated_at",
        timing: "BEFORE",
        event: "UPDATE",
        functionName: "touch_updated_at",
      }),
    ).toBe(
      'CREATE TRIGGER "users_updated_at" BEFORE UPDATE ON "public"."users" FOR EACH ROW EXECUTE FUNCTION touch_updated_at();',
    );
  });

  it("formats array and JSON cell editor payloads", () => {
    expect(postgresArrayLiteral(["a", "b'b", null])).toBe(
      "ARRAY['a', 'b''b', NULL]",
    );
    expect(formatJsonCell({ a: 1 })).toBe('{\n  "a": 1\n}');
  });

  it("builds PostGIS geometry preview SQL", () => {
    expect(geometryPreviewSql("public", "places", "geom")).toBe(
      'SELECT ST_AsGeoJSON("geom") AS geometry FROM "public"."places" WHERE "geom" IS NOT NULL LIMIT 500;',
    );
  });
});
