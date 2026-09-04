import { describe, expect, it } from "vitest";

import { objectDdlRefreshScope } from "./object-ddl";
import {
  buildTableDesignerOps,
  newTableDesignerForm,
  splitSqlExpressionList,
  tableDesignerFieldForOpIndex,
  validateTableDesignerForm,
  validatePersistedTableDesignerDraft,
} from "./table-designer";

describe("table designer", () => {
  it("orders create, table comment, column comments, and indexes", () => {
    const form = newTableDesignerForm("public");
    form.name = "accounts";
    form.comment = "Customer accounts";
    form.columns[0]!.comment = "Stable identifier";
    form.uniques.push({
      id: "unique-1",
      name: "accounts_id_key",
      columns: ["id"],
    });
    form.checks.push({
      id: "check-1",
      name: null,
      expression: "id > 0",
    });
    form.foreignKeys.push({
      id: "foreign-key-1",
      name: null,
      columns: ["id"],
      referencedSchema: " audit ",
      referencedTable: " owners ",
      referencedColumns: ["id"],
      onUpdate: "no-action",
      onDelete: "cascade",
      deferrable: false,
      initiallyDeferred: false,
    });
    form.indexes.push({
      id: "idx-1",
      name: "accounts_id_idx",
      columns: ["id"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "",
      concurrently: true,
    });

    const ops = buildTableDesignerOps(form);
    expect(ops.map((op) => op.op)).toEqual([
      "createTable",
      "setComment",
      "setComment",
      "createIndex",
    ]);
    expect(ops[3]).toMatchObject({
      op: "createIndex",
      concurrently: true,
    });
    expect(ops[0]).toMatchObject({
      op: "createTable",
      uniques: [{ name: "accounts_id_key", columns: ["id"] }],
      checks: [{ name: null, expression: "id > 0" }],
      foreignKeys: [
        expect.objectContaining({
          referencedSchema: "audit",
          referencedTable: "owners",
        }),
      ],
    });
    expect(objectDdlRefreshScope(ops)).toEqual({
      catalog: true,
      revalidateAllDescriptions: false,
      references: [
        {
          kind: "table",
          schema: "public",
          name: "accounts",
          identityArgs: null,
        },
      ],
    });
    expect(tableDesignerFieldForOpIndex(form, 2)).toBe("columns.0.comment");
    expect(tableDesignerFieldForOpIndex(form, 3)).toBe("indexes.0");
    expect(tableDesignerFieldForOpIndex(form, 0)).toBe("table");
    expect(tableDesignerFieldForOpIndex(form, 99)).toBe("table");
  });

  it("normalizes identity columns to not-null without a default", () => {
    const form = newTableDesignerForm("public");
    form.name = "accounts";
    form.columns[0] = {
      ...form.columns[0]!,
      nullable: true,
      defaultKind: "expression",
      defaultValue: "42",
      identity: "always",
    };

    expect(buildTableDesignerOps(form)[0]).toMatchObject({
      op: "createTable",
      columns: [{ nullable: false, default: null, identity: "always" }],
    });
  });

  it("distinguishes an empty literal default from no default", () => {
    const form = newTableDesignerForm("public");
    form.name = "labels";
    form.columns[0] = {
      ...form.columns[0]!,
      identity: "none",
      defaultKind: "literal",
      defaultValue: "",
    };
    expect(buildTableDesignerOps(form)[0]).toMatchObject({
      op: "createTable",
      columns: [{ default: { kind: "literal", value: "" } }],
    });

    form.columns[0]!.defaultKind = "none";
    expect(buildTableDesignerOps(form)[0]).toMatchObject({
      op: "createTable",
      columns: [{ default: null }],
    });

    form.columns[0]!.defaultKind = "expression";
    expect(validateTableDesignerForm(form).fields).toMatchObject({
      "columns.0.defaultValue": "Default expression is required.",
    });
  });

  it("splits index expressions only at top-level commas", () => {
    expect(
      splitSqlExpressionList(
        `coalesce(a, b), lower("last,name"), $tag$a,b$tag$, ARRAY[1, 2]`,
      ),
    ).toEqual([
      "coalesce(a, b)",
      'lower("last,name")',
      "$tag$a,b$tag$",
      "ARRAY[1, 2]",
    ]);
    expect(splitSqlExpressionList("coalesce(a, b),")).toEqual([
      "coalesce(a, b)",
      "",
    ]);
  });

  it("reports local validation errors before preview", () => {
    const form = newTableDesignerForm("");
    form.columns.push({ ...form.columns[0]!, id: "duplicate" });
    form.foreignKeys.push({
      id: "foreign-key-1",
      name: null,
      columns: ["missing"],
      referencedSchema: "public",
      referencedTable: "parents",
      referencedColumns: ["id", "tenant_id"],
      onUpdate: "no-action",
      onDelete: "no-action",
      deferrable: false,
      initiallyDeferred: false,
    });

    expect(validateTableDesignerForm(form)).toEqual({
      valid: false,
      fields: expect.objectContaining({
        schema: "Schema is required.",
        name: "Table name is required.",
        "columns.1.name": "Column id is declared more than once.",
        "foreignKeys.0.columns": "Column missing is not declared.",
        "foreignKeys.0.referencedColumns":
          "Local and referenced column counts must match.",
      }),
    });
  });

  it("rejects unsafe embedded SQL at the exact designer fields", () => {
    const form = newTableDesignerForm("public");
    form.name = "accounts";
    form.columns[0] = {
      ...form.columns[0]!,
      identity: "none",
      dataType: "integer NOT NULL",
      defaultKind: "expression",
      defaultValue: "0, DROP COLUMN secret",
    };
    form.checks.push({
      id: "check-1",
      name: null,
      expression: "id > 0; DROP TABLE accounts",
    });
    form.indexes.push({
      id: "index-1",
      name: "accounts_id_idx",
      columns: ["id; DROP TABLE accounts"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "id > 0; DROP TABLE accounts",
      concurrently: false,
    });

    expect(validateTableDesignerForm(form)).toEqual({
      valid: false,
      fields: {
        "columns.0.dataType": "Data type cannot contain column options.",
        "columns.0.defaultValue": "Default expression escapes its field.",
        "checks.0.expression":
          "Check expression cannot contain a statement boundary.",
        "indexes.0.columns":
          "Index expression cannot contain a statement boundary.",
        "indexes.0.wherePredicate":
          "Index predicate cannot contain a statement boundary.",
      },
    });
  });

  it("accepts common PostgreSQL fragments that keep renderer delimiters intact", () => {
    const form = newTableDesignerForm("public");
    form.name = "measurements";
    form.columns[0] = {
      ...form.columns[0]!,
      identity: "none",
      dataType: "numeric(12, 2)[]",
      defaultKind: "expression",
      defaultValue: "ARRAY[1, 2]",
    };
    form.checks.push({
      id: "check-1",
      name: null,
      expression: "label <> $tag$a, b$tag$",
    });

    expect(validateTableDesignerForm(form)).toEqual({
      valid: true,
      fields: {},
    });
  });

  it("accepts PostgreSQL escape strings without weakening fragment boundaries", () => {
    const form = newTableDesignerForm("public");
    form.name = "measurements";
    form.columns[0] = {
      ...form.columns[0]!,
      identity: "none",
      defaultKind: "expression",
      defaultValue: "E'a\\'b'",
    };
    form.checks.push({
      id: "check-1",
      name: null,
      expression: "label <> E'a\\'b'",
    });

    expect(validateTableDesignerForm(form)).toEqual({
      valid: true,
      fields: {},
    });

    form.checks[0]!.expression = "label <> E'a\\'b'; DROP TABLE measurements";
    expect(validateTableDesignerForm(form).fields).toMatchObject({
      "checks.0.expression":
        "Check expression cannot contain a statement boundary.",
    });

    form.checks[0]!.expression = "label <> 'a\\'' OR true";
    expect(validateTableDesignerForm(form).fields).toMatchObject({
      "checks.0.expression": "Check expression escapes its field.",
    });
  });

  it("never conditionally creates a table before follow-up operations", () => {
    const form = newTableDesignerForm("public");
    form.name = "accounts";
    form.comment = "Existing tables must not be mutated";
    form.indexes.push({
      id: "index-1",
      name: "accounts_id_idx",
      columns: ["id"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "",
      concurrently: false,
    });
    const legacyPersistedDraft = { ...form, ifNotExists: true };

    const restored = validatePersistedTableDesignerDraft(
      legacyPersistedDraft,
      "public",
    );
    expect(restored).not.toHaveProperty("ifNotExists");
    expect(buildTableDesignerOps(restored ?? form)[0]).toMatchObject({
      op: "createTable",
      ifNotExists: false,
    });
  });

  it("validates a complete persisted draft and rejects malformed or cross-schema state", () => {
    const form = newTableDesignerForm("audit");
    form.name = "events";
    form.checks.push({ id: "check-1", name: null, expression: "id > 0" });
    form.indexes.push({
      id: "idx-1",
      name: "events_id_idx",
      columns: ["id"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "",
      concurrently: false,
    });

    expect(validatePersistedTableDesignerDraft(form, "audit")).toEqual(form);
    expect(
      validatePersistedTableDesignerDraft(
        { ...form, columns: [{ ...form.columns[0], nullable: "no" }] },
        "audit",
      ),
    ).toBeUndefined();
    expect(validatePersistedTableDesignerDraft(form, "public")).toBeUndefined();
  });

  it("rejects an unsafe predicate restored from a persisted draft", () => {
    const form = newTableDesignerForm("audit");
    form.name = "events";
    form.indexes.push({
      id: "idx-1",
      name: "events_id_idx",
      columns: ["id"],
      unique: false,
      method: "btree",
      include: [],
      wherePredicate: "active; DROP TABLE audit.events",
      concurrently: false,
    });

    const restored = validatePersistedTableDesignerDraft(form, "audit");
    expect(restored).toEqual(form);
    expect(validateTableDesignerForm(restored!).fields).toMatchObject({
      "indexes.0.wherePredicate":
        "Index predicate cannot contain a statement boundary.",
    });
  });
});
