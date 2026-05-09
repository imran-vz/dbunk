import { describe, expect, it } from "vitest";

import {
  type ColumnChangeKind,
  classifyDestructive,
  generatePostgresDdl,
} from "@/lib/ddl/postgres";

describe("generatePostgresDdl", () => {
  it("returns an empty string when there are no changes", () => {
    expect(generatePostgresDdl("public", "users", [])).toBe("");
  });

  it("emits ADD COLUMN with type and NOT NULL", () => {
    const change: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "email",
        dataType: "text",
        nullable: false,
        defaultValue: null,
      },
    };

    const sql = generatePostgresDdl("public", "users", [change]);

    expect(sql).toBe(
      'ALTER TABLE "public"."users" ADD COLUMN "email" text NOT NULL;',
    );
  });

  it("emits ADD COLUMN with NULL when nullable", () => {
    const change: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "nickname",
        dataType: "varchar(64)",
        nullable: true,
        defaultValue: null,
      },
    };

    const sql = generatePostgresDdl("public", "users", [change]);

    expect(sql).toBe(
      'ALTER TABLE "public"."users" ADD COLUMN "nickname" varchar(64);',
    );
  });

  it("quotes string defaults and escapes single quotes", () => {
    const change: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "label",
        dataType: "text",
        nullable: false,
        defaultValue: "it's fine",
      },
    };

    const sql = generatePostgresDdl("public", "users", [change]);

    expect(sql).toBe(
      'ALTER TABLE "public"."users" ADD COLUMN "label" text NOT NULL DEFAULT \'it\'\'s fine\';',
    );
  });

  it("leaves numeric literal defaults unquoted", () => {
    const change: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "score",
        dataType: "integer",
        nullable: false,
        defaultValue: "42",
      },
    };

    const sql = generatePostgresDdl("public", "users", [change]);

    expect(sql).toBe(
      'ALTER TABLE "public"."users" ADD COLUMN "score" integer NOT NULL DEFAULT 42;',
    );
  });

  it("leaves function-call defaults unquoted", () => {
    const change: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "created_at",
        dataType: "timestamptz",
        nullable: false,
        defaultValue: "now()",
      },
    };

    const sql = generatePostgresDdl("public", "users", [change]);

    expect(sql).toBe(
      'ALTER TABLE "public"."users" ADD COLUMN "created_at" timestamptz NOT NULL DEFAULT now();',
    );
  });

  it("emits DROP COLUMN", () => {
    const change: ColumnChangeKind = {
      kind: "drop",
      columnName: "legacy",
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" DROP COLUMN "legacy";',
    );
  });

  it("emits RENAME COLUMN as its own statement", () => {
    const change: ColumnChangeKind = {
      kind: "rename",
      columnName: "old_name",
      newName: "new_name",
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" RENAME COLUMN "old_name" TO "new_name";',
    );
  });

  it("emits ALTER COLUMN ... TYPE for set_type", () => {
    const change: ColumnChangeKind = {
      kind: "set_type",
      columnName: "score",
      newType: "integer",
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" ALTER COLUMN "score" TYPE integer;',
    );
  });

  it("emits SET NOT NULL when nullable=false", () => {
    const change: ColumnChangeKind = {
      kind: "set_nullable",
      columnName: "name",
      nullable: false,
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" ALTER COLUMN "name" SET NOT NULL;',
    );
  });

  it("emits DROP NOT NULL when nullable=true", () => {
    const change: ColumnChangeKind = {
      kind: "set_nullable",
      columnName: "name",
      nullable: true,
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" ALTER COLUMN "name" DROP NOT NULL;',
    );
  });

  it("emits SET DEFAULT with quoted string", () => {
    const change: ColumnChangeKind = {
      kind: "set_default",
      columnName: "status",
      default: "pending",
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" ALTER COLUMN "status" SET DEFAULT \'pending\';',
    );
  });

  it("emits DROP DEFAULT when default is null", () => {
    const change: ColumnChangeKind = {
      kind: "set_default",
      columnName: "status",
      default: null,
    };
    expect(generatePostgresDdl("public", "users", [change])).toBe(
      'ALTER TABLE "public"."users" ALTER COLUMN "status" DROP DEFAULT;',
    );
  });

  it("escapes embedded double quotes in identifiers", () => {
    const change: ColumnChangeKind = {
      kind: "drop",
      columnName: 'weird"name',
    };
    expect(generatePostgresDdl('pub"lic', 'us"ers', [change])).toBe(
      'ALTER TABLE "pub""lic"."us""ers" DROP COLUMN "weird""name";',
    );
  });

  it("joins multiple statements with newlines preserving order", () => {
    const sql = generatePostgresDdl("public", "users", [
      {
        kind: "add",
        column: {
          name: "email",
          dataType: "text",
          nullable: true,
          defaultValue: null,
        },
      },
      { kind: "drop", columnName: "legacy" },
      { kind: "rename", columnName: "old", newName: "new" },
    ]);
    expect(sql).toBe(
      [
        'ALTER TABLE "public"."users" ADD COLUMN "email" text;',
        'ALTER TABLE "public"."users" DROP COLUMN "legacy";',
        'ALTER TABLE "public"."users" RENAME COLUMN "old" TO "new";',
      ].join("\n"),
    );
  });

  it("treats decimal numeric defaults as unquoted literals", () => {
    const change: ColumnChangeKind = {
      kind: "set_default",
      columnName: "price",
      default: "12.50",
    };
    expect(generatePostgresDdl("public", "products", [change])).toBe(
      'ALTER TABLE "public"."products" ALTER COLUMN "price" SET DEFAULT 12.50;',
    );
  });
});

describe("classifyDestructive", () => {
  it("returns empty arrays for no input", () => {
    expect(classifyDestructive([])).toEqual({
      destructive: [],
      nonDestructive: [],
    });
  });

  it("flags drop column as destructive", () => {
    const change: ColumnChangeKind = { kind: "drop", columnName: "legacy" };
    const result = classifyDestructive([change]);
    expect(result.destructive).toEqual([change]);
    expect(result.nonDestructive).toEqual([]);
  });

  it("flags set_type as destructive", () => {
    const change: ColumnChangeKind = {
      kind: "set_type",
      columnName: "score",
      newType: "integer",
    };
    const result = classifyDestructive([change]);
    expect(result.destructive).toEqual([change]);
    expect(result.nonDestructive).toEqual([]);
  });

  it("flags set_nullable=false as destructive", () => {
    const change: ColumnChangeKind = {
      kind: "set_nullable",
      columnName: "name",
      nullable: false,
    };
    const result = classifyDestructive([change]);
    expect(result.destructive).toEqual([change]);
    expect(result.nonDestructive).toEqual([]);
  });

  it("does not flag set_nullable=true (relaxing nullability) as destructive", () => {
    const change: ColumnChangeKind = {
      kind: "set_nullable",
      columnName: "name",
      nullable: true,
    };
    const result = classifyDestructive([change]);
    expect(result.destructive).toEqual([]);
    expect(result.nonDestructive).toEqual([change]);
  });

  it("treats add, rename, and set_default as non-destructive", () => {
    const changes: ColumnChangeKind[] = [
      {
        kind: "add",
        column: {
          name: "email",
          dataType: "text",
          nullable: true,
          defaultValue: null,
        },
      },
      { kind: "rename", columnName: "old", newName: "new" },
      { kind: "set_default", columnName: "status", default: "pending" },
      { kind: "set_default", columnName: "status", default: null },
    ];
    const result = classifyDestructive(changes);
    expect(result.destructive).toEqual([]);
    expect(result.nonDestructive).toEqual(changes);
  });

  it("partitions a mixed list correctly", () => {
    const drop: ColumnChangeKind = { kind: "drop", columnName: "legacy" };
    const add: ColumnChangeKind = {
      kind: "add",
      column: {
        name: "email",
        dataType: "text",
        nullable: true,
        defaultValue: null,
      },
    };
    const setNotNull: ColumnChangeKind = {
      kind: "set_nullable",
      columnName: "name",
      nullable: false,
    };
    const result = classifyDestructive([drop, add, setNotNull]);
    expect(result.destructive).toEqual([drop, setNotNull]);
    expect(result.nonDestructive).toEqual([add]);
  });
});
