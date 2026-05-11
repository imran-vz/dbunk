import { describe, expect, it } from "vitest";

import { generateClickHouseDdl } from "./clickhouse";

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
