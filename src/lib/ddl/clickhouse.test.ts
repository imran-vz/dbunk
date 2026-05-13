import { describe, expect, it } from "vitest";

import { generateClickHouseDdl, renderChange } from "./clickhouse";

describe("generateClickHouseDdl", () => {
  it("returns empty for no changes", () => {
    expect(generateClickHouseDdl("analytics", "users", [])).toBe("");
  });

  it("emits ADD COLUMN with backtick identifiers and the literal type", () => {
    const sql = generateClickHouseDdl("analytics", "users", [
      {
        kind: "add",
        column: {
          name: "country",
          dataType: "String",
          nullable: false,
          defaultValue: null,
        },
      },
    ]);
    expect(sql).toBe(
      "ALTER TABLE `analytics`.`users` ADD COLUMN `country` String;",
    );
  });

  it("wraps a nullable add-column type with Nullable() unless already wrapped", () => {
    const sql = generateClickHouseDdl("analytics", "users", [
      {
        kind: "add",
        column: {
          name: "middle_name",
          dataType: "String",
          nullable: true,
          defaultValue: null,
        },
      },
      {
        kind: "add",
        column: {
          name: "score",
          dataType: "Nullable(Int64)",
          nullable: true,
          defaultValue: null,
        },
      },
    ]);
    expect(sql).toBe(
      [
        "ALTER TABLE `analytics`.`users` ADD COLUMN `middle_name` Nullable(String);",
        "ALTER TABLE `analytics`.`users` ADD COLUMN `score` Nullable(Int64);",
      ].join("\n"),
    );
  });

  it("emits ADD COLUMN with a numeric default literal", () => {
    const sql = generateClickHouseDdl("analytics", "events", [
      {
        kind: "add",
        column: {
          name: "amount",
          dataType: "Decimal(18,4)",
          nullable: false,
          defaultValue: "0",
        },
      },
    ]);
    expect(sql).toBe(
      "ALTER TABLE `analytics`.`events` ADD COLUMN `amount` Decimal(18,4) DEFAULT 0;",
    );
  });

  it("quotes string defaults and escapes embedded quotes", () => {
    const sql = generateClickHouseDdl("analytics", "events", [
      {
        kind: "add",
        column: {
          name: "label",
          dataType: "String",
          nullable: false,
          defaultValue: "o'brien",
        },
      },
    ]);
    expect(sql).toBe(
      "ALTER TABLE `analytics`.`events` ADD COLUMN `label` String DEFAULT 'o''brien';",
    );
  });

  it("emits DROP COLUMN, RENAME COLUMN, and MODIFY COLUMN forms", () => {
    const sql = generateClickHouseDdl("analytics", "users", [
      { kind: "drop", columnName: "legacy" },
      { kind: "rename", columnName: "uname", newName: "user_name" },
      { kind: "set_type", columnName: "score", newType: "Int64" },
    ]);
    expect(sql).toBe(
      [
        "ALTER TABLE `analytics`.`users` DROP COLUMN `legacy`;",
        "ALTER TABLE `analytics`.`users` RENAME COLUMN `uname` TO `user_name`;",
        "ALTER TABLE `analytics`.`users` MODIFY COLUMN `score` Int64;",
      ].join("\n"),
    );
  });

  it("translates set_nullable using the supplied current-type map", () => {
    const types = new Map([
      ["score", "Int64"],
      ["legacy", "Nullable(String)"],
    ]);
    const sql = generateClickHouseDdl(
      "analytics",
      "users",
      [
        { kind: "set_nullable", columnName: "score", nullable: true },
        { kind: "set_nullable", columnName: "legacy", nullable: false },
      ],
      types,
    );
    expect(sql).toBe(
      [
        "ALTER TABLE `analytics`.`users` MODIFY COLUMN `score` Nullable(Int64);",
        "ALTER TABLE `analytics`.`users` MODIFY COLUMN `legacy` String;",
      ].join("\n"),
    );
  });

  it("comments out a set_nullable change when no current type is available", () => {
    const sql = generateClickHouseDdl("analytics", "users", [
      { kind: "set_nullable", columnName: "missing_meta", nullable: true },
    ]);
    expect(sql).toMatch(/^-- ALTER TABLE/);
    expect(sql).toContain("type not loaded");
  });

  it("supports SET DEFAULT and REMOVE DEFAULT shapes", () => {
    const sql = generateClickHouseDdl("analytics", "events", [
      { kind: "set_default", columnName: "label", default: "fresh" },
      { kind: "set_default", columnName: "amount", default: null },
    ]);
    expect(sql).toBe(
      [
        "ALTER TABLE `analytics`.`events` MODIFY COLUMN `label` DEFAULT 'fresh';",
        "ALTER TABLE `analytics`.`events` MODIFY COLUMN `amount` REMOVE DEFAULT;",
      ].join("\n"),
    );
  });

  it("escapes backticks in identifier names", () => {
    const sql = generateClickHouseDdl("my`schema", "tbl", [
      { kind: "drop", columnName: "weird`col" },
    ]);
    expect(sql).toBe(
      "ALTER TABLE `my``schema`.`tbl` DROP COLUMN `weird``col`;",
    );
  });
});

describe("renderChange", () => {
  const empty = new Map<string, string>();

  it("renders an add-column change with a non-nullable bare type", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        {
          kind: "add",
          column: {
            name: "country",
            dataType: "String",
            nullable: false,
            defaultValue: null,
          },
        },
        empty,
      ),
    ).toBe("ALTER TABLE `analytics`.`users` ADD COLUMN `country` String;");
  });

  it("wraps a nullable add-column type with Nullable() when not already wrapped", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        {
          kind: "add",
          column: {
            name: "score",
            dataType: "Int64",
            nullable: true,
            defaultValue: null,
          },
        },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`users` ADD COLUMN `score` Nullable(Int64);",
    );
  });

  it("preserves an already-wrapped Nullable() type on add-column", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        {
          kind: "add",
          column: {
            name: "score",
            dataType: "Nullable(Int64)",
            nullable: true,
            defaultValue: null,
          },
        },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`users` ADD COLUMN `score` Nullable(Int64);",
    );
  });

  it("strips Nullable() when adding a non-nullable column with a wrapped type", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        {
          kind: "add",
          column: {
            name: "score",
            dataType: "Nullable(Int64)",
            nullable: false,
            defaultValue: null,
          },
        },
        empty,
      ),
    ).toBe("ALTER TABLE `analytics`.`users` ADD COLUMN `score` Int64;");
  });

  it("emits a numeric DEFAULT literal unquoted on add-column", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        {
          kind: "add",
          column: {
            name: "amount",
            dataType: "Decimal(18,4)",
            nullable: false,
            defaultValue: "0",
          },
        },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` ADD COLUMN `amount` Decimal(18,4) DEFAULT 0;",
    );
  });

  it("emits a function-call DEFAULT unquoted on add-column", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        {
          kind: "add",
          column: {
            name: "created_at",
            dataType: "DateTime",
            nullable: false,
            defaultValue: "now()",
          },
        },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` ADD COLUMN `created_at` DateTime DEFAULT now();",
    );
  });

  it("quotes string DEFAULTs and escapes single quotes on add-column", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        {
          kind: "add",
          column: {
            name: "label",
            dataType: "String",
            nullable: false,
            defaultValue: "o'brien",
          },
        },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` ADD COLUMN `label` String DEFAULT 'o''brien';",
    );
  });

  it("renders a drop-column change", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        { kind: "drop", columnName: "legacy" },
        empty,
      ),
    ).toBe("ALTER TABLE `analytics`.`users` DROP COLUMN `legacy`;");
  });

  it("renders a rename-column change", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        { kind: "rename", columnName: "uname", newName: "user_name" },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`users` RENAME COLUMN `uname` TO `user_name`;",
    );
  });

  it("renders a set_type change by forwarding the literal new type", () => {
    expect(
      renderChange(
        "analytics",
        "users",
        { kind: "set_type", columnName: "score", newType: "Nullable(Int64)" },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`users` MODIFY COLUMN `score` Nullable(Int64);",
    );
  });

  it("wraps the current type when set_nullable=true and the type is known", () => {
    const types = new Map([["score", "Int64"]]);
    expect(
      renderChange(
        "analytics",
        "users",
        { kind: "set_nullable", columnName: "score", nullable: true },
        types,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`users` MODIFY COLUMN `score` Nullable(Int64);",
    );
  });

  it("unwraps the current type when set_nullable=false and the type is known", () => {
    const types = new Map([["legacy", "Nullable(String)"]]);
    expect(
      renderChange(
        "analytics",
        "users",
        { kind: "set_nullable", columnName: "legacy", nullable: false },
        types,
      ),
    ).toBe("ALTER TABLE `analytics`.`users` MODIFY COLUMN `legacy` String;");
  });

  it("emits a wrap-with-Nullable comment when set_nullable=true and the type is missing", () => {
    const result = renderChange(
      "analytics",
      "users",
      { kind: "set_nullable", columnName: "missing_meta", nullable: true },
      empty,
    );
    expect(result.startsWith("-- ALTER TABLE")).toBe(true);
    expect(result).toContain("wrap with");
    expect(result).toContain("type not loaded");
  });

  it("emits a remove-Nullable comment when set_nullable=false and the type is missing", () => {
    const result = renderChange(
      "analytics",
      "users",
      { kind: "set_nullable", columnName: "missing_meta", nullable: false },
      empty,
    );
    expect(result.startsWith("-- ALTER TABLE")).toBe(true);
    expect(result).toContain("remove");
    expect(result).toContain("type not loaded");
  });

  it("renders set_default with a string value as a quoted DEFAULT", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        { kind: "set_default", columnName: "label", default: "fresh" },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` MODIFY COLUMN `label` DEFAULT 'fresh';",
    );
  });

  it("renders set_default with a numeric value as an unquoted DEFAULT", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        { kind: "set_default", columnName: "amount", default: "12.50" },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` MODIFY COLUMN `amount` DEFAULT 12.50;",
    );
  });

  it("renders set_default with null as REMOVE DEFAULT", () => {
    expect(
      renderChange(
        "analytics",
        "events",
        { kind: "set_default", columnName: "amount", default: null },
        empty,
      ),
    ).toBe(
      "ALTER TABLE `analytics`.`events` MODIFY COLUMN `amount` REMOVE DEFAULT;",
    );
  });
});
