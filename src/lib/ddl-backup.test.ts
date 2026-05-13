import { describe, expect, it } from "vitest";
import {
  buildCrossConnectionCopySql,
  buildPgDumpArgs,
  buildPgRestoreArgs,
  ddlExportStatement,
} from "@/lib/ddl-backup";

const connection = {
  host: "localhost",
  port: 5432,
  database: "app",
  user: "imran",
};

describe("DDL and backup planning", () => {
  it("builds table, schema, and database DDL export statements", () => {
    expect(
      ddlExportStatement({ kind: "table", schema: "public", table: "users" }),
    ).toContain("public.users");
    expect(ddlExportStatement({ kind: "schema", schema: "audit" })).toContain(
      "audit",
    );
    expect(ddlExportStatement({ kind: "database" })).toBe(
      "SELECT pg_get_ddl_for_database();",
    );
  });

  it("builds pg_dump arguments for custom table dumps", () => {
    expect(
      buildPgDumpArgs({
        connection,
        scope: { kind: "table", schema: "public", table: "users" },
        format: "custom",
        file: "/tmp/users.dump",
      }),
    ).toEqual([
      "--host",
      "localhost",
      "--port",
      "5432",
      "--username",
      "imran",
      "--dbname",
      "app",
      "--file",
      "/tmp/users.dump",
      "--format",
      "custom",
      "--table",
      "public.users",
    ]);
  });

  it("builds pg_restore arguments", () => {
    expect(
      buildPgRestoreArgs({
        connection,
        file: "/tmp/app.dump",
        clean: true,
        dataOnly: true,
      }),
    ).toContain("--data-only");
  });

  it("builds cross-connection COPY statements", () => {
    expect(
      buildCrossConnectionCopySql({
        sourceSchema: "public",
        sourceTable: "users",
        targetSchema: "archive",
        targetTable: "users",
        columns: ["id", "full name"],
      }),
    ).toEqual({
      exportSql:
        'COPY (SELECT "id", "full name" FROM "public"."users") TO STDOUT WITH (FORMAT csv, HEADER true)',
      importSql:
        'COPY "archive"."users" ("id", "full name") FROM STDIN WITH (FORMAT csv, HEADER true)',
    });
  });
});
